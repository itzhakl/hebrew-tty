#!/usr/bin/env node
'use strict';

const os = require('os');
const patch = require('../src/patch');

const USAGE = `rtl-caret - put the terminal caret on the glyph it is editing in RTL text

  rtl-caret status              show what is installed, change nothing
  sudo rtl-caret install        patch the editor's WebGL renderer
  sudo rtl-caret uninstall      restore the backups

  --app <path>   extra application root to search, repeatable

Editor bundles are replaced by upgrades, so re-run install afterwards.
`;

function parse(argv) {
  const opts = { cmd: null, apps: [] };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--app') opts.apps.push(argv[++i]);
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
  const { cmd, apps } = parse(process.argv.slice(2));
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
      const hasBackup = require('fs').existsSync(patch.backupOf(file));
      console.log(`${patch.stateOf(file).padEnd(12)} ${hasBackup ? 'backup' : 'no backup'}  ${file}`);
    }
    return 0;
  }

  if (cmd === 'install') {
    needRoot();
    let failed = 0;
    for (const file of targets) {
      const r = patch.applyTo(file);
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
