import { once } from 'node:events'
import { appendFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { setTimeout as sleep } from 'node:timers/promises'
import { parseArgs } from 'node:util'
import { ImageSink } from './images.ts'
import { formatPageChange, type PageChange, PageTimer } from './pagetime.ts'
import { type BenchKeys, benchControlIds, summariseLine } from './protocol.ts'
import { executeCommand } from './repl.ts'
import { DEFAULT_OPTIONS, FakeDeckSession } from './session.ts'

/** Key size on a bench surface unless --key-size says otherwise: the ScreenKey's 128 px. */
const BENCH_KEY_SIZE = 128

const CLI_DEFAULTS = {
	host: '127.0.0.1',
	port: String(DEFAULT_OPTIONS.port),
	keySize: String(DEFAULT_OPTIONS.keySize),
	format: DEFAULT_OPTIONS.bitmapFormat,
	deviceId: 'fake-deck-01',
	out: 'out',
}

const HELP = `Usage: node src/main.ts [options]

Registers a fake deck (two LCD keys + one 128x64 encoder/OLED) with Companion over Satellite TCP,
or with --keys a 16- or 32-key bench surface like the firmware's (THI-76).

Options:
  --host <host>         Companion host (default: ${CLI_DEFAULTS.host})
  --port <port>         Satellite TCP port (default: ${CLI_DEFAULTS.port})
  --key-size <px>       Square key bitmap size (default: ${CLI_DEFAULTS.keySize}, or ${BENCH_KEY_SIZE} with --keys)
  --keys <16|32>        Register a bench surface: rows of 8 keys from 0/0, no encoder; logs one line per page change
  --format <rgb|png>    Requested bitmap format (default: ${CLI_DEFAULTS.format})
  --device-id <id>      Satellite DEVICEID (default: ${CLI_DEFAULTS.deviceId})
  --out <dir>           Output directory for PNGs and protocol.log (default: ${CLI_DEFAULTS.out})
  --save-raw            Also save each image as received: <id>.wire.png (png) or <id>.rgb (rgb)
  --no-ping             Send no PING and disable the silence watchdog (measures the server idle timeout)
  -h, --help            Show this help

REPL commands:
  press <0|1>           Press and release key 0/0 or 0/1 (page up / down on the bench pages)
  push                  Press and release the encoder
  rotate <+n|-n>        Rotate the encoder by n detents (one KEY-ROTATE DIRECTION=±1 each)
  quit                  Send QUIT and exit
`

function usageErrorFor(format: string | null, port: number, keySize: number, keys: number | null): string | null {
	if (!format) return '--format must be rgb or png'
	if (keys === null) return '--keys must be 16 or 32'
	if (!Number.isInteger(port) || port < 1 || port > 65535) return '--port must be an integer from 1 to 65535'
	if (!Number.isInteger(keySize) || keySize < 1) return '--key-size must be a positive integer'
	return null
}

function main(): void {
	const { values } = parseArgs({
		options: {
			host: { type: 'string', default: CLI_DEFAULTS.host },
			port: { type: 'string', default: CLI_DEFAULTS.port },
			'key-size': { type: 'string' },
			keys: { type: 'string' },
			'save-raw': { type: 'boolean', default: false },
			format: { type: 'string', default: CLI_DEFAULTS.format },
			'device-id': { type: 'string', default: CLI_DEFAULTS.deviceId },
			out: { type: 'string', default: CLI_DEFAULTS.out },
			'no-ping': { type: 'boolean', default: false },
			help: { type: 'boolean', short: 'h', default: false },
		},
	})
	if (values.help) {
		process.stdout.write(HELP)
		return
	}
	const port = Number(values.port)
	const keys: BenchKeys | null =
		values.keys === undefined ? 0 : values.keys === '16' ? 16 : values.keys === '32' ? 32 : null
	const keySize = Number(values['key-size'] ?? (keys ? BENCH_KEY_SIZE : CLI_DEFAULTS.keySize))
	const format = values.format === 'rgb' || values.format === 'png' ? values.format : null
	const usageError = usageErrorFor(format, port, keySize, keys)
	if (usageError || !format || keys === null) {
		console.error(usageError)
		process.exit(2)
	}

	const outDir = values.out
	mkdirSync(outDir, { recursive: true })
	const logFile = join(outDir, 'protocol.log')
	let logBroken = false
	const record = (text: string) => {
		if (logBroken) return
		try {
			appendFileSync(logFile, `${new Date().toISOString()} ${text}\n`)
		} catch (err) {
			logBroken = true
			console.error(`protocol.log disabled: ${(err as Error).message}`)
		}
	}

	const session = new FakeDeckSession({
		...DEFAULT_OPTIONS,
		host: values.host,
		port,
		deviceId: values['device-id'],
		keySize,
		keys,
		bitmapFormat: format,
		ping: !values['no-ping'],
	})
	const sink = new ImageSink({
		outDir,
		keySize,
		keys,
		saveRaw: values['save-raw'],
		quiet: keys > 0,
		log: (l) => {
			record(`   ${l}`)
			console.log(l)
		},
	})

	session.on('line', (dir: 'in' | 'out', line: string) => record(`${dir === 'in' ? '<-' : '->'} ${summariseLine(line)}`))
	session.on('log', (l: string) => {
		record(`## ${l}`)
		console.log(`# ${l}`)
	})
	const timer = keys ? new PageTimer(keys) : null
	const benchIds = benchControlIds(keys)
	const report = (change: PageChange | null) => {
		if (!change || !timer) return
		const line = formatPageChange(change, format)
		record(`## ${line}`)
		console.log(`# ${line}`)
		if (timer.summaryDue()) {
			const summary = `page changes: press to last key median ${timer.median()} ms, max ${timer.max()} ms`
			record(`## ${summary}`)
			console.log(`# ${summary}`)
		}
	}
	session.on('keyState', (params, timing) => {
		const result = sink.handleKeyState(params)
		if (!timer || !result.ok) return
		report(
			timer.drawn({
				key: benchIds.indexOf(String(params.CONTROLID)),
				lineStartedMs: timing.startedAt,
				doneMs: Date.now(),
				wireBytes: timing.bytes,
				receiveMs: timing.receivedAt - timing.startedAt,
				decodeMs: result.decodeMs,
			})
		)
	})
	if (timer) {
		session.on('line', (dir: 'in' | 'out', line: string) => {
			if (dir === 'out' && line.startsWith('KEY-PRESS ') && / PRESSED=1\b/.test(line)) timer.pressed(Date.now())
		})
		setInterval(() => report(timer.tick(Date.now())), 100).unref()
	}
	session.on('clear', () => sink.clear())
	session.on('fatal', () => process.exit(1))

	const rl = createInterface({ input: process.stdin, output: process.stdout, prompt: 'deck> ' })
	const registered = once(session, 'registered')
	let closed = false
	// readline does not await async handlers; chain commands so a press's release is sent before the next
	// command, and hold input (e.g. piped) until the deck has registered. `quit` never waits.
	let pending = Promise.resolve()
	rl.on('line', (input) => {
		pending = pending
			.then(async () => {
				if (input.trim() !== 'quit') await registered
				const keepGoing = await executeCommand(session, input)
				if (closed) return
				if (keepGoing) rl.prompt()
				else rl.close()
			})
			.catch((err: Error) => console.error(`command failed: ${err.message}`))
	})
	rl.on('close', async () => {
		closed = true
		await Promise.race([pending, sleep(DEFAULT_OPTIONS.silenceTimeoutMs)])
		session.stop()
		setTimeout(() => process.exit(0), 300)
	})

	record(`## start ${JSON.stringify(session.opts)}`)
	session.start()
	rl.prompt()
}

if (import.meta.main) main()
