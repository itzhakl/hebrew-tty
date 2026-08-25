#!/usr/bin/env node
'use strict';

/* tools/patch-binary.py finds its edit sites in the Claude Code executable by
 * the shape of the code, because the minifier renames every local on each
 * build. binary-anchors.json holds those sites as they were recorded from two
 * real binaries whose builds share no identifier at all, so a pattern that
 * stops resolving is caught here rather than in a 340MB write. */

const path = require('path');
const { spawnSync } = require('child_process');

const patcher = path.join(__dirname, '..', 'tools', 'patch-binary.py');
const fixture = path.join(__dirname, 'fixtures', 'binary-anchors.json');

console.log('binary anchors: every edit site resolves without knowing a name');
const run = spawnSync('python3', [patcher, '--selftest', fixture], {
  encoding: 'utf8',
});

if (run.error && run.error.code === 'ENOENT') {
  console.log('  python3 not found, skipped');
  process.exit(0);
}

process.stdout.write(run.stdout || '');
process.stderr.write(run.stderr || '');
process.exit(run.status === 0 ? 0 : 1);
