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
const {
  ElevenLabsProvider,
  parseElevenLabsCredential,
  mapElevenLabsError,
  buildRealtimeUrl,
  normalizeLanguage
} = require('../src/voice/elevenlabs');
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

function testCredentials() {
  eq(parseElevenLabsCredential(' sk_abc123 '), 'sk_abc123', 'the key is trimmed');
  throwsWith(() => parseElevenLabsCredential(''), 'No ElevenLabs API key', 'empty credential');
  // The two things a migration off Google actually leaves in the box.
  throwsWith(() => parseElevenLabsCredential('AIzaSyExample'), 'Google Cloud API key',
    'a leftover Google API key is named, not just rejected');
  throwsWith(() => parseElevenLabsCredential('{"project_id":"x"}'), 'service-account JSON',
    'a leftover service account is named, not just rejected');
  throwsWith(() => parseElevenLabsCredential('sk_a sk_b'), 'whitespace', 'a mangled paste is caught');

  eq(normalizeLanguage('iw-IL'), 'he', 'the tag the Google config carried still means Hebrew');
  eq(normalizeLanguage('he'), 'he', 'a bare code passes through');
  eq(normalizeLanguage('ar-SA'), 'ar', 'the region is dropped');
  eq(normalizeLanguage(''), '', 'no language is not a language');

  const url = new URL(buildRealtimeUrl({ languageCode: 'iw-IL' }));
  eq(url.host, 'api.elevenlabs.io', 'the default host');
  eq(url.pathname, '/v1/speech-to-text/realtime', 'the realtime path');
  eq(url.searchParams.get('model_id'), 'scribe_v2_realtime', 'the default model');
  eq(url.searchParams.get('audio_format'), 'pcm_16000', 'declares linear16 at 16 kHz');
  eq(url.searchParams.get('language_code'), 'he', 'the language is normalized into the URL');
  // Without server-side endpointing, committed_transcript never fires for a
  // microphone and the whole utterance would ride on our own flush.
  eq(url.searchParams.get('commit_strategy'), 'vad', 'server-side endpointing is on');

  eq(mapElevenLabsError({ message_type: 'auth_error', error: 'bad key' }).hint, 'set-api-key', 'auth hint');
  eq(mapElevenLabsError({ message_type: 'quota_exceeded', error: 'no credits' }).hint, 'quota', 'quota hint');
  eq(mapElevenLabsError(new Error('getaddrinfo ENOTFOUND api.elevenlabs.io')).hint, 'network', 'network hint');
  eq(mapElevenLabsError({ message_type: 'transcriber_error', error: 'something else' }).hint, undefined,
    'unknown errors carry no hint');
  ok(mapElevenLabsError({ message_type: 'auth_error', error: 'bad key' }).message.includes('bad key'),
    'the raw server text survives');
}

// ---------- config ----------

function testConfig() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rtl-voice-'));
  const file = path.join(dir, 'voice.json');

  const empty = config.load({}, {}, file);
  eq(empty.port, 8765, 'default port');
  eq(empty.language, 'he', 'Hebrew is the default language');
  eq(empty.provider, 'elevenlabs', 'elevenlabs is the default provider');
  eq(empty.model, 'scribe_v2_realtime', 'the realtime Scribe model is the default');

  config.save({ credential: 'sk_saved', port: 9000 }, file);
  eq(fs.statSync(file).mode & 0o777, 0o600, 'config file is 0600');
  const saved = config.load({}, {}, file);
  eq(saved.port, 9000, 'file overrides the default');
  eq(saved.credential, 'sk_saved', 'credential read back from the file');
  eq(saved.language, 'he', 'unsaved keys keep their defaults');

  const env = config.load({}, { RTL_VOICE_PORT: '9100', ELEVENLABS_API_KEY: 'sk_env' }, file);
  eq(env.port, 9100, 'environment overrides the file');
  eq(env.credential, 'sk_env', 'inline env credential wins over the file');
  eq(config.load({}, { XI_API_KEY: 'sk_xi' }, file).credential, 'sk_xi', "ElevenLabs' own variable is honoured");

  const flags = config.load({ port: 9200, language: 'ar-SA' }, { RTL_VOICE_PORT: '9100' }, file);
  eq(flags.port, 9200, 'explicit flags override the environment');
  eq(flags.language, 'ar-SA', 'language flag applies');

  eq(config.load({ provider: 'nonsense' }, {}, file).provider, 'elevenlabs', 'unknown provider falls back');

  // A voice.json left over from the Google backend must not send "long" as a
  // Scribe model_id, nor keep selecting an engine that is gone.
  config.save({ provider: 'hybrid', model: 'long', location: 'eu' }, file);
  const migrated = config.load({}, {}, file);
  eq(migrated.provider, 'elevenlabs', 'the retired provider is replaced');
  eq(migrated.model, 'scribe_v2_realtime', 'the retired model is replaced');

  fs.writeFileSync(file, '{not json');
  throwsWith(() => config.load({}, {}, file), 'not valid JSON', 'corrupt config is reported, not swallowed');
  fs.rmSync(dir, { recursive: true, force: true });
}

// ---------- CLI parsing ----------

function testCliParse() {
  const run = cli.parse(['--port', '9000', '--', 'claude', '--continue']);
  eq(run.overrides.port, 9000, 'port flag parsed');
  eq(run.command.join(' '), 'claude --continue', 'everything after -- is the command');

  const model = cli.parse(['serve', '--model', 'scribe_v2_realtime']);
  eq(model.overrides.model, 'scribe_v2_realtime', 'model flag parsed');

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
  const base = config.load({ credential: 'sk_test' }, {}, '/nonexistent');
  eq(cli.buildProvider(base).id, 'elevenlabs', 'the elevenlabs provider is built');
  throwsWith(() => cli.buildProvider(Object.assign({}, base, { credential: '' })),
    'No ElevenLabs API key', 'a missing credential fails before the mic opens');
  throwsWith(() => cli.buildProvider(Object.assign({}, base, { credential: 'AIzaSyOld' })),
    'not an ElevenLabs API key', 'a leftover Google key fails before the mic opens');
}

// ---------- fake engines ----------

/* Stands in for the realtime WebSocket, so the provider and the socket can be
 * driven without ElevenLabs. */
function fakeSocket() {
  const listeners = {};
  return {
    sent: [],
    closed: false,
    on(event, fn) {
      (listeners[event] = listeners[event] || []).push(fn);
      return this;
    },
    send(text) {
      this.sent.push(JSON.parse(text));
    },
    close() {
      this.closed = true;
      this.emit('close');
    },
    emit(event, arg) {
      for (const fn of listeners[event] || []) fn(arg);
    },
    reply(obj) {
      this.emit('message', Buffer.from(JSON.stringify(obj)));
    }
  };
}

/* chunkMs 20 makes one CLI frame a full chunk, so batching does not have to be
 * spelled out in every assertion below. */
function elevenWith(socket, opts) {
  return new ElevenLabsProvider(
    Object.assign({ credential: 'sk_test', languageCode: 'iw-IL', chunkMs: 20 }, opts),
    async () => socket
  );
}

async function testElevenLabsSession() {
  const socket = fakeSocket();
  const interims = [];
  const finals = [];
  const session = await elevenWith(socket).createSession({
    onInterim: (t) => interims.push(t),
    onFinal: (t) => finals.push(t),
    onError: (e) => finals.push(`ERR ${e.message}`)
  });

  socket.reply({ message_type: 'session_started', session_id: 'abc' });
  eq(interims.length, 0, 'the session handshake is not a transcript');
  socket.reply({ message_type: 'partial_transcript', text: 'שלום' });
  eq(interims[0], 'שלום', 'a partial surfaces as an interim');
  // final_transcript is a settled hypothesis, not a commit. Treating it as one
  // as well as committed_transcript would send the same words twice.
  socket.reply({ message_type: 'final_transcript', text: 'שלום עולם' });
  eq(interims[1], 'שלום עולם', 'a settled final is still only a hypothesis');
  eq(finals.length, 0, 'final_transcript does not commit');
  socket.reply({ message_type: 'committed_transcript', text: 'שלום עולם' });
  eq(finals[0], 'שלום עולם', 'committed_transcript is the commit');

  session.sendAudio(Buffer.alloc(640));
  eq(socket.sent.length, 1, 'a full chunk is shipped');
  eq(socket.sent[0].message_type, 'input_audio_chunk', 'audio goes out as input_audio_chunk');
  eq(socket.sent[0].sample_rate, 16000, 'declares the CLI sample rate');
  eq(socket.sent[0].commit, false, 'audio alone does not commit');
  eq(Buffer.from(socket.sent[0].audio_base_64, 'base64').length, 640, 'the audio survives base64');

  session.sendAudio(Buffer.alloc(320));
  eq(socket.sent.length, 1, 'a part chunk is held back');
  session.flush();
  eq(socket.sent.length, 2, 'flush ships the tail');
  eq(socket.sent[1].commit, true, 'flush commits the segment instead of waiting out the silence');
  eq(Buffer.from(socket.sent[1].audio_base_64, 'base64').length, 320, 'the held audio rides the commit');
  session.sendAudio(Buffer.alloc(640));
  eq(socket.sent.length, 2, 'no audio is written after flush');

  socket.reply({ message_type: 'committed_transcript', text: 'הזנב' });
  eq(finals[1], 'הזנב', 'the flushed tail commits');
  eq(await session.endSegment(), '', 'push mode commits through onFinal, not endSegment');

  // A commit we asked for and never got must not hang the close path.
  const slow = fakeSocket();
  const slowSession = await elevenWith(slow, { settleTimeoutMs: 100 }).createSession({
    onInterim: () => {},
    onFinal: () => {},
    onError: () => {}
  });
  slowSession.sendAudio(Buffer.alloc(640));
  slowSession.flush();
  const waited = Date.now();
  eq(await slowSession.endSegment(), '', 'an unanswered commit falls through');
  ok(Date.now() - waited >= 90, 'endSegment waits for the commit it asked for');

  // An engine that dies must not throw once per remaining audio frame.
  const dying = fakeSocket();
  const errors = [];
  const dyingSession = await elevenWith(dying).createSession({
    onInterim: () => {},
    onFinal: () => {},
    onError: (e) => errors.push(e)
  });
  dying.reply({ message_type: 'quota_exceeded', error: 'out of credits' });
  eq(errors.length, 1, 'the error is reported once');
  eq(errors[0].hint, 'quota', 'the error is mapped');
  dyingSession.sendAudio(Buffer.alloc(640));
  dyingSession.sendAudio(Buffer.alloc(640));
  eq(dying.sent.length, 0, 'a dead socket takes no further audio');

  // A throttled commit is answered by the server's own VAD a moment later.
  // Painting TranscriptError over live text would lose the transcript.
  const throttled = fakeSocket();
  const throttledErrors = [];
  const throttledSession = await elevenWith(throttled).createSession({
    onInterim: () => {},
    onFinal: () => {},
    onError: (e) => throttledErrors.push(e)
  });
  throttled.reply({ message_type: 'commit_throttled', error: 'too soon' });
  eq(throttledErrors.length, 0, 'a throttled commit is not painted over the transcript');
  throttledSession.sendAudio(Buffer.alloc(640));
  eq(throttled.sent.length, 1, 'and the session stays alive');
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
 * provider session costs a TLS handshake and an authenticated upgrade on the
 * first mic press. Frames that arrive in that window must be replayed, not
 * dropped - dropping them eats the first words of the utterance. */
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
        throw new Error('ElevenLabs rejected the credential');
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
  await testElevenLabsSession();
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
