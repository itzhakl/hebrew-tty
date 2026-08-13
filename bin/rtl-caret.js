#!/usr/bin/env node
'use strict';

const os = require('os');
const patch = require('../src/patch');

const USAGE = `rtl-caret - put the terminal caret on the glyph it is editing in RTL text

  rtl-caret status              show what is installed, change nothing
  sudo rtl-caret install        patch the editor's WebGL renderer
  sudo rtl-caret uninstall      restore the backups

  --align        also flush rows that start in Hebrew to the right edge
  --no-mirror    do not apply bidi mirroring to brackets in RTL runs
  --app <path>   extra application root to search, repeatable

Editor bundles are replaced by upgrades, so re-run install afterwards.
`;

function parse(argv) {
  const opts = { cmd: null, apps: [], align: false, mirror: true };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--app') opts.apps.push(argv[++i]);
    else if (argv[i] === '--align') opts.align = true;
    else if (argv[i] === '--no-mirror') opts.mirror = false;
    else if (!opts.cmd) opts.cmd = argv[i];
  }
  return opts;
}

function needRoot() {
  if (typeof process.getuid === 'function' && process.getuid() !== 0) {
    console.error('needs root: re-run with sudo');
    process.exit(1);
  }
}

function main() {
  const { cmd, apps, align, mirror } = parse(process.argv.slice(2));
  if (!cmd || cmd === 'help' || cmd === '--help' || cmd === '-h') {
    process.stdout.write(USAGE);
    return 0;
  }

  const targets = patch.discover(apps);
  if (!targets.length) {
    console.error('no VS Code / VSCodium installation found.');
    console.error('pass one with --app <path to resources/app>');
    return 1;
  }

  if (cmd === 'status') {
    for (const file of targets) {
      const fs = require('fs');
      const hasBackup = fs.existsSync(patch.backupOf(file));
      const state = patch.stateOf(file);
      const src = state === 'patched' ? fs.readFileSync(file, 'utf8') : '';
      const on = ['caret'];
      if (src.includes(patch.MIRROR_FLAG)) on.push('mirror');
      if (src.includes(patch.ALIGN_FLAG)) on.push('align');
      console.log(
        `${state.padEnd(12)} ${(state === 'patched' ? on.join('+') : '').padEnd(19)} ` +
        `${hasBackup ? 'backup' : 'no backup'}  ${file}`
      );
    }
    return 0;
  }

  if (cmd === 'install') {
    needRoot();
    let failed = 0;
    for (const file of targets) {
      const r = patch.applyTo(file, { align, mirror });
      if (!r.ok) failed++;
      console.log(`${r.ok ? 'ok  ' : 'skip'}  ${r.note}  ${file}`);
    }
    console.log(`\nrestart the editor completely for this to take effect (${os.platform()})`);
    return failed && failed === targets.length ? 1 : 0;
  }

  if (cmd === 'uninstall') {
    needRoot();
    for (const file of targets) {
      const r = patch.revertFile(file);
      console.log(`${r.ok ? 'ok  ' : 'skip'}  ${r.note}  ${file}`);
    }
    return 0;
  }

  console.error(`unknown command: ${cmd}`);
  process.stdout.write(USAGE);
  return 1;
}

process.exit(main());
