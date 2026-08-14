#!/usr/bin/env node
/* Drive a real VSCodium under CDP and report what the patched renderer does.
 *
 * The pty probes in this directory capture the bytes Claude writes. They cannot
 * see the renderer, which is where the patch lives, so this one launches a
 * throwaway editor instance, opens a terminal, runs Claude in it, pastes text,
 * and reads __rtlLog and a screenshot back out of the window.
 *
 *   node tools/editor-probe.js paste "שלום עולם"
 *   node tools/editor-probe.js dictate "היום אני" " רוצה לבדוק"
 *   node tools/editor-probe.js shell 'echo "שלום עולם"'
 *
 * Requires an X or Wayland session and a patched editor. The instance uses its
 * own user-data-dir, so nothing touches the editor you are working in.
 */
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn, execFileSync } = require('child_process');

const PORT = Number(process.env.RTL_CDP_PORT || 9333);
const ROOT = path.join(os.tmpdir(), 'rtl-caret-probe');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/* ---- CDP, over Node's global WebSocket ---------------------------------- */

class Client {
  constructor(ws) {
    this.ws = ws;
    this.id = 0;
    this.pending = new Map();
    ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(ev.data);
      const p = this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id);
      if (msg.error) p.reject(new Error(JSON.stringify(msg.error)));
      else p.resolve(msg.result);
    });
  }

  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((resolve, reject) => {
      ws.addEventListener('open', resolve, { once: true });
      ws.addEventListener('error', reject, { once: true });
    });
    return new Client(ws);
  }

  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }

  async eval(expression) {
    const r = await this.send('Runtime.evaluate', {
      expression, returnByValue: true, awaitPromise: true
    });
    if (r.exceptionDetails) {
      throw new Error(r.exceptionDetails.exception?.description || 'eval failed');
    }
    return r.result.value;
  }

  async key(key, { code, modifiers = 0, text } = {}) {
    const base = { key, code: code || key, modifiers };
    await this.send('Input.dispatchKeyEvent', {
      ...base, type: text ? 'keyDown' : 'rawKeyDown', text
    });
    await this.send('Input.dispatchKeyEvent', { ...base, type: 'keyUp' });
  }

  close() { this.ws.close(); }
}

async function attach(timeoutMs = 60000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const hit = list.find((x) => x.type === 'page' && /VSCodium|Visual Studio Code/.test(x.title));
      if (hit) {
        const c = await Client.connect(hit.webSocketDebuggerUrl);
        await c.send('Runtime.enable');
        await c.send('Page.enable');
        return c;
      }
    } catch (e) { /* not up yet */ }
    await sleep(500);
  }
  throw new Error('no editor window answered on the debugging port');
}

/* ---- the editor instance ------------------------------------------------ */

function launch() {
  for (const d of ['udd', 'ext', 'proj']) fs.mkdirSync(path.join(ROOT, d), { recursive: true });
  const child = spawn('codium', [
    `--remote-debugging-port=${PORT}`,
    `--user-data-dir=${path.join(ROOT, 'udd')}`,
    `--extensions-dir=${path.join(ROOT, 'ext')}`,
    '--disable-workspace-trust',
    '--new-window',
    path.join(ROOT, 'proj')
  ], { detached: true, stdio: 'ignore' });
  child.unref();
}

async function running() {
  try {
    await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
    return true;
  } catch (e) {
    return false;
  }
}

/* ---- terminal actions --------------------------------------------------- */

const pasteExpr = (text) => `(() => {
  const ta = document.querySelector('.xterm-helper-textarea');
  if (!ta) return 'no terminal';
  ta.focus();
  const dt = new DataTransfer();
  dt.setData('text/plain', ${JSON.stringify(text)});
  ta.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
  return 'pasted';
})()`;

async function openTerminal(c) {
  if (await c.eval('!!document.querySelector(".xterm-helper-textarea")')) return;
  await c.key('`', { code: 'Backquote', modifiers: 2 });
  await sleep(6000);
}

async function clearPrompt(c, n = 200) {
  for (let i = 0; i < n; i++) await c.key('Backspace', { code: 'Backspace' });
  await sleep(1500);
}

async function report(c, label) {
  const entries = JSON.parse(await c.eval('JSON.stringify(__rtlLog)'));
  console.log(`\n--- ${label} ---`);
  for (const e of entries) {
    const rec = e.recovered === undefined ? '' : `\n      recovered ${JSON.stringify(e.recovered)}`;
    console.log(
      `  ${e.kind} row ${e.row} shift ${e.shift} caret ${e.caret}->${e.mapped}\n` +
      `      painted   ${JSON.stringify(e.text.replace(/\s+$/, ''))}${rec}`
    );
  }
  fs.mkdirSync(ROOT, { recursive: true });
  const file = path.join(ROOT, `${label}.png`);
  const shot = await c.send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(file, Buffer.from(shot.data, 'base64'));
  console.log(`  screenshot ${file}`);
}

/* Drag across the row the renderer last painted, copy it, and read back what
 * the terminal actually handed the clipboard. Wayland only, which is what this
 * machine runs; the point is the round trip, not portability. */
async function copyRow(c) {
  const last = JSON.parse(await c.eval('JSON.stringify(__rtlLog.slice(-1)[0] || null)'));
  if (!last) return console.log('  nothing painted to copy');

  // Under WebGL there are no row elements to measure. xterm parks its hidden
  // textarea on the cursor cell, which is the row being edited - the one worth
  // copying - so its position is the row position.
  const geom = JSON.parse(await c.eval(`(() => {
    const r = document.querySelector('.xterm-screen').getBoundingClientRect();
    const t = document.querySelector('.xterm-helper-textarea').getBoundingClientRect();
    return JSON.stringify({ x: r.left, w: r.width, y: t.top + t.height / 2 });
  })()`));

  const y = geom.y;
  const drag = (type, x) =>
    c.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons: 1, clickCount: 1 });
  await drag('mousePressed', geom.x + 2);
  await drag('mouseMoved', geom.x + geom.w - 2);
  await drag('mouseReleased', geom.x + geom.w - 2);
  await sleep(800);
  await c.key('c', { code: 'KeyC', modifiers: 2 });
  await sleep(1200);

  const clip = execFileSync('wl-paste', { encoding: 'utf8' }).replace(/\s+$/, '');
  console.log(`  painted   ${JSON.stringify(last.text.replace(/\s+$/, ''))}`);
  console.log(`  clipboard ${JSON.stringify(clip)}`);
}

/* ---- scenarios ---------------------------------------------------------- */

async function main() {
  const [mode, ...args] = process.argv.slice(2);
  if (!mode) {
    console.error('usage: editor-probe.js <paste|dictate|shell> <text...>');
    process.exit(2);
  }

  if (!(await running())) {
    launch();
    console.log('launched a throwaway editor instance');
  }
  const c = await attach();
  await openTerminal(c);

  if (mode === 'shell') {
    // A fresh terminal, so the text goes to a plain shell rather than to
    // whatever is already running in the one that is open.
    await c.key('`', { code: 'Backquote', modifiers: 2 | 8 });
    await sleep(5000);
    await c.eval('__rtlLog.length = 0');
    await c.send('Input.insertText', { text: args.join(' ') });
    await c.key('Enter', { code: 'Enter', text: '\r' });
    await sleep(2500);
    await report(c, 'shell');
    c.close();
    return;
  }

  // Both remaining modes talk to Claude, so make sure it is up.
  // The tab title follows the foreground process, so it says "Claude Code"
  // exactly when Claude already owns the terminal.
  const hasClaude = await c.eval(
    '[".single-terminal-tab",".terminal-tabs-container",".pane-header .title"]' +
    '.map(s=>document.querySelector(s)?.innerText||"").join(" ")'
  );
  if (!/Claude/i.test(hasClaude)) {
    // Flush whatever is half-typed on the shell line; a bad command is fine.
    await c.key('Enter', { code: 'Enter', text: '\r' });
    await sleep(1200);
    await c.send('Input.insertText', { text: 'claude' });
    await c.key('Enter', { code: 'Enter', text: '\r' });
    await sleep(20000);
    await c.key('Enter', { code: 'Enter', text: '\r' }); // trust prompt, if shown
    await sleep(4000);
  }

  await clearPrompt(c);
  await c.eval('__rtlLog.length = 0');

  if (mode === 'paste') {
    console.log(await c.eval(pasteExpr(args.join(' '))));
    await sleep(3500);
    await report(c, 'paste');
  } else if (mode === 'dictate') {
    for (let i = 0; i < args.length; i++) {
      console.log(await c.eval(pasteExpr(args[i])));
      await sleep(2500);
      await report(c, `dictate-${i + 1}`);
    }
  } else if (mode === 'copy') {
    console.log(await c.eval(pasteExpr(args.join(' '))));
    await sleep(3500);
    await report(c, 'copy');
    await copyRow(c);
  } else {
    console.error(`unknown mode ${mode}`);
    process.exit(2);
  }
  c.close();
}

main().catch((e) => {
  console.error(`FAIL ${e.message}`);
  process.exit(1);
});
