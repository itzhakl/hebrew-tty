#!/usr/bin/env node
'use strict';

/* Dictation has no pty recordings to check against: the protocol is Claude's,
 * not ours. So the socket round-trip below drives the real server with a fake
 * provider, which is the closest thing to a fixture we can hold - the frames
 * are exactly what the CLI sends and expects. */

const fs = require('fs');
const os = require('os');
const path = require('path');

const { Endpointer } = require('../src/voice/vad');
const { parseGoogleCredential, mapChirpError, speechEndpoint, ChirpProvider } = require('../src/voice/chirp');
const { HybridProvider } = require('../src/voice/hybrid');
const config = require('../src/voice/config');
const server = require('../src/voice/server');
const cli = require('../src/voice/cli');

let failures = 0;
let checks = 0;

function ok(cond, msg) {
  checks++;
  if (!cond) {
    failures++;
    console.log(`  FAIL ${msg}`);
  }
}

function eq(actual, expected, msg) {
  ok(actual === expected, `${msg}: got ${JSON.stringify(actual)}, want ${JSON.stringify(expected)}`);
}

function throwsWith(fn, fragment, msg) {
  try {
    fn();
    ok(false, `${msg}: expected a throw`);
  } catch (e) {
    ok(String(e.message).includes(fragment), `${msg}: got "${e.message}"`);
  }
}

const SAMPLE_RATE = 16000;
function frame(ms, amplitude) {
  const n = (SAMPLE_RATE * ms) / 1000;
  const pcm = new Int16Array(n);
  for (let i = 0; i < n; i++) pcm[i] = i % 2 ? amplitude : -amplitude;
  return pcm;
}
const LOUD = 8000;
const SILENT = 0;

// ---------- VAD ----------

function testVad() {
  const ep = new Endpointer();
  eq(ep.pushFrame(frame(400, LOUD)).commit, undefined, 'speech alone does not commit');
  ok(ep.speaking, 'speaking after loud frame');
  eq(ep.pushFrame(frame(500, SILENT)).commit, undefined, 'silence under endpointMs holds');
  eq(ep.pushFrame(frame(200, SILENT)).commit, 'silence', 'silence past endpointMs commits');
  ok(!ep.speaking, 'reset after commit');

  const short = new Endpointer();
  short.pushFrame(frame(100, LOUD));
  const r = short.pushFrame(frame(700, SILENT));
  eq(r.commit, undefined, 'utterance under minUtteranceMs is not committed');
  eq(r.discarded, true, 'utterance under minUtteranceMs is discarded');

  // The extension capped segments at 4 s, which chops long Hebrew sentences.
  const capped = new Endpointer({ maxSegmentMs: 12000 });
  for (let i = 0; i < 39; i++) {
    eq(capped.pushFrame(frame(300, LOUD)).commit, undefined, `continuous speech at ${i * 300}ms holds`);
  }
  eq(capped.pushFrame(frame(300, LOUD)).commit, 'max-segment', 'commits at maxSegmentMs');

  const quiet = new Endpointer();
  eq(quiet.pushFrame(frame(2000, SILENT)).commit, undefined, 'silence before any speech never commits');
  ok(!quiet.speaking, 'silence alone does not open a segment');
}

// ---------- credentials ----------

const SERVICE_ACCOUNT = JSON.stringify({
  project_id: 'my-project-1',
  client_email: 'a@b.iam.gserviceaccount.com',
  private_key: '-----BEGIN PRIVATE KEY-----x'
});

function testCredentials() {
  eq(speechEndpoint('global'), 'speech.googleapis.com', 'global endpoint');
  eq(speechEndpoint('eu'), 'eu-speech.googleapis.com', 'regional endpoint');

  const key = parseGoogleCredential('AIzaSyExample', 'my-project-1', 'eu');
  eq(key.projectId, 'my-project-1', 'api key keeps the configured project');
  eq(key.clientOptions.apiKey, 'AIzaSyExample', 'api key is passed through');
  eq(key.clientOptions.apiEndpoint, 'eu-speech.googleapis.com', 'endpoint follows location');

  // The JSON's own project wins: a stale configured id must not redirect a
  // valid service account at a foreign project.
  const sa = parseGoogleCredential(SERVICE_ACCOUNT, 'other-project', 'us');
  eq(sa.projectId, 'my-project-1', 'service-account project_id wins');
  ok(sa.clientOptions.credentials !== undefined, 'service account passes credentials');

  throwsWith(() => parseGoogleCredential('', undefined, 'eu'), 'No Google Cloud credential', 'empty credential');
  throwsWith(() => parseGoogleCredential('AIza', undefined, 'eu'), 'needs a project ID', 'api key without project');
  throwsWith(() => parseGoogleCredential('AIza', 'AIzaSyNotAProject', 'eu'), 'not a valid Google Cloud project ID',
    'api key pasted into the project box');
  throwsWith(() => parseGoogleCredential('{oops', undefined, 'eu'), 'not valid', 'broken JSON');
  throwsWith(() => parseGoogleCredential('{"project_id":"my-project-1"}', undefined, 'eu'),
    'client_email', 'JSON missing the key material');

  eq(mapChirpError({ code: 7, message: 'denied on recognizer' }).hint, 'set-api-key', 'permission denied hint');
  eq(mapChirpError({ code: 8, message: 'quota' }).hint, 'quota', 'quota hint');
  eq(mapChirpError({ code: 14, message: 'unavailable' }).hint, 'network', 'network hint');
  eq(mapChirpError({ message: 'something else' }).hint, undefined, 'unknown errors carry no hint');
  ok(mapChirpError({ code: 7, message: 'denied on recognizer' }).message.includes('denied on recognizer'),
    'the raw server text survives');
}

// ---------- config ----------

function testConfig() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rtl-voice-'));
  const file = path.join(dir, 'voice.json');

  const empty = config.load({}, {}, file);
  eq(empty.port, 8765, 'default port');
  eq(empty.language, 'iw-IL', 'Hebrew is the default language');
  eq(empty.provider, 'hybrid', 'hybrid is the default provider');

  config.save({ credential: 'AIzaSaved', projectId: 'my-project-1', port: 9000 }, file);
  eq(fs.statSync(file).mode & 0o777, 0o600, 'config file is 0600');
  const saved = config.load({}, {}, file);
  eq(saved.port, 9000, 'file overrides the default');
  eq(saved.credential, 'AIzaSaved', 'credential read back from the file');
  eq(saved.language, 'iw-IL', 'unsaved keys keep their defaults');

  const env = config.load({}, { RTL_VOICE_PORT: '9100', GOOGLE_STT_CREDENTIAL: 'AIzaEnv' }, file);
  eq(env.port, 9100, 'environment overrides the file');
  eq(env.credential, 'AIzaEnv', 'inline env credential wins over the file');

  const keyFile = path.join(dir, 'sa.json');
  fs.writeFileSync(keyFile, `${SERVICE_ACCOUNT}\n`);
  const viaFile = config.load({}, { GOOGLE_APPLICATION_CREDENTIALS: keyFile }, file);
  eq(viaFile.credential, SERVICE_ACCOUNT, 'credential read from GOOGLE_APPLICATION_CREDENTIALS');

  const flags = config.load({ port: 9200, language: 'ar-SA' }, { RTL_VOICE_PORT: '9100' }, file);
  eq(flags.port, 9200, 'explicit flags override the environment');
  eq(flags.language, 'ar-SA', 'language flag applies');

  eq(config.load({ provider: 'nonsense' }, {}, file).provider, 'hybrid', 'unknown provider falls back');

  fs.writeFileSync(file, '{not json');
  throwsWith(() => config.load({}, {}, file), 'not valid JSON', 'corrupt config is reported, not swallowed');
  fs.rmSync(dir, { recursive: true, force: true });
}

// ---------- CLI parsing ----------

function testCliParse() {
  const run = cli.parse(['--port', '9000', '--', 'claude', '--continue']);
  eq(run.overrides.port, 9000, 'port flag parsed');
  eq(run.command.join(' '), 'claude --continue', 'everything after -- is the command');

  const serve = cli.parse(['serve', '--verbose']);
  eq(serve.sub, 'serve', 'subcommand parsed');
  eq(serve.verbose, true, 'verbose parsed');
  eq(serve.command.length, 0, 'no command without --');

  // A wrapped command carrying its own flags must not be re-parsed as ours.
  const nested = cli.parse(['--', 'claude', '--port', '1234']);
  eq(nested.overrides.port, undefined, "the wrapped command's flags stay its own");
  eq(nested.command.length, 3, 'wrapped command keeps its arguments');
}

// ---------- provider selection ----------

function testProviderSelection() {
  const base = config.load({ credential: 'AIzaX', projectId: 'my-project-1' }, {}, '/nonexistent');
  eq(cli.buildProvider(Object.assign({}, base, { provider: 'chirp' })).id, 'chirp', 'chirp selected');
  eq(cli.buildProvider(Object.assign({}, base, { provider: 'hybrid' })).id, 'hybrid', 'hybrid selected');
  throwsWith(() => cli.buildProvider(Object.assign({}, base, { credential: '' })),
    'No Google Cloud credential', 'a missing credential fails before the mic opens');
}

// ---------- fake engines ----------

/* Stands in for the gRPC duplex stream, so the provider and the socket can be
 * driven without Google. */
function fakeStream() {
  const listeners = {};
  return {
    writes: [],
    ended: false,
    on(event, fn) {
      (listeners[event] = listeners[event] || []).push(fn);
      return this;
    },
    write(req) {
      this.writes.push(req);
      return true;
    },
    end() {
      this.ended = true;
      for (const fn of listeners.end || []) fn();
    },
    emit(event, arg) {
      for (const fn of listeners[event] || []) fn(arg);
    }
  };
}

function chirpWith(stream, opts) {
  return new ChirpProvider(
    Object.assign({ credential: 'AIzaX', projectId: 'my-project-1', location: 'eu', model: 'long', languageCode: 'iw-IL' }, opts),
    async () => ({ stream, close: async () => {} })
  );
}

function result(transcript, isFinal) {
  return { results: [{ alternatives: [{ transcript }], isFinal }] };
}

async function testChirpSession() {
  const stream = fakeStream();
  const interims = [];
  const finals = [];
  const provider = chirpWith(stream);
  const session = await provider.createSession({
    onInterim: (t) => interims.push(t),
    onFinal: (t) => finals.push(t),
    onError: (e) => finals.push(`ERR ${e.message}`)
  });

  const cfg = stream.writes[0].streamingConfig.config;
  eq(cfg.explicitDecodingConfig.sampleRateHertz, 16000, 'declares the CLI sample rate');
  eq(cfg.explicitDecodingConfig.encoding, 'LINEAR16', 'declares linear16');
  eq(cfg.languageCodes[0], 'iw-IL', 'sends the configured language');
  eq(stream.writes[0].recognizer, 'projects/my-project-1/locations/eu/recognizers/_', 'recognizer path');

  stream.emit('data', result('שלום', false));
  eq(interims[0], 'שלום', 'interim surfaces');
  stream.emit('data', result('שלום עולם', true));
  eq(finals[0], 'שלום עולם', 'server-endpointed final is pushed');

  session.sendAudio(Buffer.alloc(320));
  eq(stream.writes.length, 2, 'audio is forwarded');
  session.flush();
  ok(stream.ended, 'flush half-closes the stream');
  session.sendAudio(Buffer.alloc(320));
  eq(stream.writes.length, 2, 'no audio is written after flush');
  eq(await session.endSegment(), '', 'push mode commits through onFinal, not endSegment');

  // An engine that dies must not throw once per remaining audio frame.
  const dying = fakeStream();
  const errors = [];
  const dyingSession = await chirpWith(dying).createSession({
    onInterim: () => {},
    onFinal: () => {},
    onError: (e) => errors.push(e)
  });
  dying.emit('error', { code: 8, message: 'quota' });
  eq(errors.length, 1, 'the error is reported once');
  eq(errors[0].hint, 'quota', 'the error is mapped');
  dyingSession.sendAudio(Buffer.alloc(320));
  dyingSession.sendAudio(Buffer.alloc(320));
  eq(dying.writes.length, 1, 'a dead stream takes no further audio');
}

async function testHybridSession() {
  const fastStream = fakeStream();
  const accurateStream = fakeStream();
  const provider = new HybridProvider(chirpWith(fastStream), chirpWith(accurateStream, { location: 'us', model: 'chirp_3' }));
  const interims = [];
  const session = await provider.createSession({ onInterim: (t) => interims.push(t), onError: () => {} });

  fastStream.emit('data', result('שלום', false));
  eq(interims[interims.length - 1], 'שלום', 'fast interim is displayed');
  fastStream.emit('data', result('שלום עולם', true));
  eq(interims[interims.length - 1], 'שלום עולם', 'fast final is demoted to display');
  eq(await session.endSegment(), '', 'nothing commits mid-recording');

  session.flush();
  accurateStream.emit('data', result('שלום עולם מדויק', true));
  accurateStream.emit('end');
  eq(await session.endSegment(), 'שלום עולם מדויק', 'the accurate engine owns the commit');

  // The accurate engine failing must not lose the user's dictation.
  const f2 = fakeStream();
  const a2 = fakeStream();
  const s2 = await new HybridProvider(chirpWith(f2), chirpWith(a2)).createSession({ onInterim: () => {}, onError: () => {} });
  f2.emit('data', result('גיבוי', true));
  s2.flush();
  a2.emit('error', { code: 14, message: 'unavailable' });
  eq(await s2.endSegment(), 'גיבוי', 'falls back to the fast transcript');

  // An accurate engine that dies mid-sentence has heard only part of it. Its
  // words are better and yet fewer, so committing them drops the tail.
  const f3 = fakeStream();
  const a3 = fakeStream();
  const s3 = await new HybridProvider(chirpWith(f3), chirpWith(a3)).createSession({ onInterim: () => {}, onError: () => {} });
  a3.emit('data', result('החצי הראשון', true));
  a3.emit('end');
  f3.emit('data', result('החצי הראשון והחצי השני', true));
  s3.flush();
  eq(await s3.endSegment(), 'החצי הראשון והחצי השני', 'a partial accurate transcript loses to the complete fast one');

  // But an engine that ends because we flushed it is the one we want.
  const f4 = fakeStream();
  const a4 = fakeStream();
  const s4 = await new HybridProvider(chirpWith(f4), chirpWith(a4)).createSession({ onInterim: () => {}, onError: () => {} });
  f4.emit('data', result('גרסה מהירה', true));
  s4.flush();
  a4.emit('data', result('גרסה מדויקת', true));
  a4.emit('end');
  eq(await s4.endSegment(), 'גרסה מדויקת', 'a post-flush final still wins');
}

// ---------- the socket, end to end ----------

function scriptedProvider(script) {
  return {
    id: 'scripted',
    async createSession(cb) {
      let audioFrames = 0;
      return {
        sendAudio: () => {
          audioFrames++;
          if (audioFrames === 1) cb.onInterim(script.interim);
        },
        flush: () => {},
        endSegment: async () => script.final,
        close: async () => {},
        frames: () => audioFrames
      };
    }
  };
}

async function testSocketRoundTrip() {
  const WebSocket = require('ws');
  const instance = await server.VoiceStreamServer.start({
    port: 0,
    provider: scriptedProvider({ interim: 'שלום‎', final: 'שלום עולם' }),
    makeEndpointer: () => new Endpointer()
  });
  const ws = new WebSocket(`ws://127.0.0.1:${instance.port}${server.VOICE_STREAM_PATH}`);
  const received = [];
  ws.on('message', (d) => received.push(JSON.parse(d.toString())));
  await new Promise((resolve, reject) => {
    ws.on('open', resolve);
    ws.on('error', reject);
  });

  // 20 ms of linear16 at 16 kHz, the frame size the CLI sends.
  ws.send(Buffer.alloc(640), { binary: true });
  await new Promise((r) => setTimeout(r, 50));
  eq(received.length, 1, 'the interim is sent back');
  eq(received[0].type, 'TranscriptInterim', 'a hypothesis is sent as TranscriptInterim');
  eq(received[0].data, 'שלום', 'bidi controls are stripped from the transcript');

  const closed = new Promise((r) => ws.on('close', r));
  ws.send(JSON.stringify({ type: 'KeepAlive' }));
  const closeStreamAt = Date.now();
  ws.send(JSON.stringify({ type: 'CloseStream' }));
  await closed;

  // The client arms a 1500 ms no-data timer when it sends CloseStream, and any
  // frame at all cancels it. Going quiet here costs the accurate transcript.
  const answered = received.findIndex((m, i) => i > 0 && m.type === 'TranscriptInterim');
  ok(answered !== -1, 'CloseStream is answered with a keepalive interim');

  const text = received.filter((m) => m.type === 'TranscriptText').map((m) => m.data);
  eq(text[text.length - 1], 'שלום עולם', 'the final transcript is committed on CloseStream');
  eq(received[received.length - 1].type, 'TranscriptEndpoint', 'the endpoint frame closes the utterance');
  ok(Date.now() - closeStreamAt < server.SAFETY_MS, 'the socket closes inside the client safety timeout');

  await instance.close();
}

/* Only TranscriptEndpoint commits: the client replaces its pending buffer on
 * every Text/Interim frame and promotes it on Endpoint. A commit is therefore
 * always a pair, and the text must be whole rather than a delta. */
async function testCommitIsAlwaysAPair() {
  const WebSocket = require('ws');
  const instance = await server.VoiceStreamServer.start({
    port: 0,
    provider: {
      id: 'two-utterances',
      async createSession(cb) {
        return {
          sendAudio: () => {
            cb.onFinal('משפט ראשון');
            cb.onFinal('משפט שני');
          },
          flush: () => {},
          endSegment: async () => '',
          close: async () => {}
        };
      }
    },
    makeEndpointer: () => new Endpointer()
  });
  const ws = new WebSocket(`ws://127.0.0.1:${instance.port}${server.VOICE_STREAM_PATH}`);
  const received = [];
  ws.on('message', (d) => received.push(JSON.parse(d.toString())));
  await new Promise((r) => ws.on('open', r));
  ws.send(Buffer.alloc(640), { binary: true });
  await new Promise((r) => setTimeout(r, 30));

  eq(received.length, 4, 'two utterances, two frames each');
  eq(received[0].type, 'TranscriptText', 'a commit leads with the text');
  eq(received[1].type, 'TranscriptEndpoint', 'and is closed by an endpoint');
  eq(received[2].data, 'משפט שני', 'the second utterance is sent whole, not as a delta');
  eq(received[3].type, 'TranscriptEndpoint', 'the second commit is endpointed too');
  ws.close();
  await instance.close();
}

/* The CLI starts sending audio as soon as the socket is open, but opening a
 * Chirp session costs an OAuth exchange and a gRPC channel on the first mic
 * press. Frames that arrive in that window must be replayed, not dropped -
 * dropping them eats the first words of the utterance. */
async function testSlowSessionKeepsTheFirstWords() {
  const WebSocket = require('ws');
  const frames = [];
  let release;
  const opening = new Promise((r) => (release = r));
  const instance = await server.VoiceStreamServer.start({
    port: 0,
    provider: {
      id: 'slow',
      async createSession() {
        await opening;
        return {
          sendAudio: (buf) => frames.push(buf.length),
          flush: () => {},
          endSegment: async () => 'המילים הראשונות',
          close: async () => {}
        };
      }
    },
    makeEndpointer: () => new Endpointer()
  });

  const ws = new WebSocket(`ws://127.0.0.1:${instance.port}${server.VOICE_STREAM_PATH}`);
  const received = [];
  ws.on('message', (d) => received.push(JSON.parse(d.toString())));
  await new Promise((r) => ws.on('open', r));

  for (let i = 0; i < 5; i++) ws.send(Buffer.alloc(640), { binary: true });
  await new Promise((r) => setTimeout(r, 50));
  eq(frames.length, 0, 'nothing reaches a session that has not opened');

  release();
  await new Promise((r) => setTimeout(r, 50));
  eq(frames.length, 5, 'every frame from the opening window is replayed');

  const closed = new Promise((r) => ws.on('close', r));
  ws.send(JSON.stringify({ type: 'CloseStream' }));
  await closed;
  eq(received.filter((m) => m.type === 'TranscriptText')[0].data, 'המילים הראשונות', 'the utterance survives a slow open');
  await instance.close();
}

/* An engine that shows interim text and then never finalises it would leave
 * the user watching words disappear: Claude paints interims grey and drops
 * them when the mic stops. */
async function testInterimFallbackOnClose() {
  const WebSocket = require('ws');
  const instance = await server.VoiceStreamServer.start({
    port: 0,
    provider: {
      id: 'never-finalises',
      async createSession(cb) {
        return {
          sendAudio: () => cb.onInterim('מה שראיתי על המסך'),
          flush: () => {},
          endSegment: async () => '',
          close: async () => {}
        };
      }
    },
    makeEndpointer: () => new Endpointer()
  });

  const ws = new WebSocket(`ws://127.0.0.1:${instance.port}${server.VOICE_STREAM_PATH}`);
  const received = [];
  ws.on('message', (d) => received.push(JSON.parse(d.toString())));
  await new Promise((r) => ws.on('open', r));
  ws.send(Buffer.alloc(640), { binary: true });
  await new Promise((r) => setTimeout(r, 30));

  const closed = new Promise((r) => ws.on('close', r));
  ws.send(JSON.stringify({ type: 'CloseStream' }));
  await closed;

  const text = received.filter((m) => m.type === 'TranscriptText').map((m) => m.data);
  eq(text[text.length - 1], 'מה שראיתי על המסך', 'the unfinalised interim is committed on close');
  eq(received[received.length - 1].type, 'TranscriptEndpoint', 'and it is endpointed like any commit');
  await instance.close();
}

/* The fallback must not double-commit text the engine did finalise. */
async function testNoDoubleCommit() {
  const WebSocket = require('ws');
  const instance = await server.VoiceStreamServer.start({
    port: 0,
    provider: {
      id: 'finalises',
      async createSession(cb) {
        return {
          sendAudio: () => {
            cb.onInterim('שלום');
            cb.onFinal('שלום עולם');
          },
          flush: () => {},
          endSegment: async () => '',
          close: async () => {}
        };
      }
    },
    makeEndpointer: () => new Endpointer()
  });

  const ws = new WebSocket(`ws://127.0.0.1:${instance.port}${server.VOICE_STREAM_PATH}`);
  const received = [];
  ws.on('message', (d) => received.push(JSON.parse(d.toString())));
  await new Promise((r) => ws.on('open', r));
  ws.send(Buffer.alloc(640), { binary: true });
  await new Promise((r) => setTimeout(r, 30));

  const closed = new Promise((r) => ws.on('close', r));
  ws.send(JSON.stringify({ type: 'CloseStream' }));
  await closed;

  const committed = received.filter((m) => m.type === 'TranscriptEndpoint');
  eq(committed.length, 1, 'a finalised utterance is committed exactly once');
  await instance.close();
}

async function testSocketReportsProviderFailure() {
  const WebSocket = require('ws');
  const instance = await server.VoiceStreamServer.start({
    port: 0,
    provider: {
      id: 'broken',
      createSession: async () => {
        throw new Error('Google Cloud rejected the credential');
      }
    },
    makeEndpointer: () => new Endpointer()
  });
  const ws = new WebSocket(`ws://127.0.0.1:${instance.port}${server.VOICE_STREAM_PATH}`);
  const messages = [];
  ws.on('message', (d) => messages.push(JSON.parse(d.toString())));
  await new Promise((r) => ws.on('close', r));
  eq(messages.length, 1, 'one error frame');
  eq(messages[0].type, 'TranscriptError', 'failures reach the client as TranscriptError');
  ok(messages[0].description.includes('rejected the credential'), 'the reason survives');
  await instance.close();
}

async function testHealthAndAdoption() {
  const provider = scriptedProvider({ interim: '', final: '' });
  const opts = { port: 8999, provider, makeEndpointer: () => new Endpointer() };
  const first = await server.startWithPortFallback(opts);
  eq(first.adopted, false, 'the first server binds');
  eq(await server.isOurServer(first.port), true, 'healthz identifies our server');

  const second = await server.startWithPortFallback(opts);
  eq(second.adopted, true, 'a second start adopts the first');
  eq(second.port, first.port, 'adoption reuses the port');
  eq(second.server, null, 'nothing new was bound');

  eq(await server.isOurServer(first.port + 1), false, 'an empty port is not ours');
  eq((await server.probe(first.port)).kind, 'ours', 'our own server probes as ours');
  eq((await server.probe(first.port + 1)).kind, 'none', 'an empty port probes as none');
  await first.server.close();

  /* The VS Code extension runs the same protocol on the same default port.
   * Calling that "no server" sends the user hunting for a dead socket that is
   * alive and serving the CLI perfectly well. */
  const http = require('http');
  const foreign = http.createServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ app: 'claude-code-voice', protocol: 'voice_stream', provider: 'hybrid', pid: 1234 }));
  });
  await new Promise((r) => foreign.listen(0, '127.0.0.1', r));
  const foreignPort = foreign.address().port;
  const probed = await server.probe(foreignPort);
  eq(probed.kind, 'foreign', "another app's voice_stream server is reported, not hidden");
  eq(probed.health.app, 'claude-code-voice', 'its identity is carried back');
  eq(await server.isOurServer(foreignPort), false, 'but it is never adopted as ours');
  await new Promise((r) => foreign.close(r));
}

// ---------- run ----------

async function main() {
  testVad();
  testCredentials();
  testConfig();
  testCliParse();
  testProviderSelection();
  await testChirpSession();
  await testHybridSession();
  await testSocketRoundTrip();
  await testCommitIsAlwaysAPair();
  await testSlowSessionKeepsTheFirstWords();
  await testInterimFallbackOnClose();
  await testNoDoubleCommit();
  await testSocketReportsProviderFailure();
  await testHealthAndAdoption();

  console.log(`voice: ${checks - failures}/${checks} checks passed`);
  if (failures) {
    console.log(`voice: ${failures} FAILED`);
    process.exit(1);
  }
}

main().catch((e) => {
  console.log(`voice: crashed - ${e && e.stack ? e.stack : e}`);
  process.exit(1);
});
