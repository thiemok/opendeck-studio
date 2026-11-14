import { appendFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { setTimeout as sleep } from 'node:timers/promises'
import { parseArgs } from 'node:util'
import { ImageSink } from './images.ts'
import { summariseLine } from './protocol.ts'
import { executeCommand } from './repl.ts'
import { DEFAULT_OPTIONS, FakeDeckSession } from './session.ts'

const CLI_DEFAULTS = {
	host: '127.0.0.1',
	port: String(DEFAULT_OPTIONS.port),
	keySize: String(DEFAULT_OPTIONS.keySize),
	format: DEFAULT_OPTIONS.bitmapFormat,
	deviceId: 'fake-deck-01',
	out: 'out',
}

const HELP = `Usage: node src/main.ts [options]

Registers a fake deck (two LCD keys + one 128x64 encoder/OLED) with Companion over Satellite TCP.

Options:
  --host <host>         Companion host (default: ${CLI_DEFAULTS.host})
  --port <port>         Satellite TCP port (default: ${CLI_DEFAULTS.port})
  --key-size <px>       Square key bitmap size (default: ${CLI_DEFAULTS.keySize})
  --format <rgb|png>    Requested bitmap format (default: ${CLI_DEFAULTS.format})
  --device-id <id>      Satellite DEVICEID (default: ${CLI_DEFAULTS.deviceId})
  --out <dir>           Output directory for PNGs and protocol.log (default: ${CLI_DEFAULTS.out})
  --no-ping             Send no PING and disable the silence watchdog (measures the server idle timeout)
  -h, --help            Show this help

REPL commands:
  press <0|1>           Press and release LCD key 0 or 1
  push                  Press and release the encoder
  rotate <+n|-n>        Rotate the encoder by n detents (one KEY-ROTATE DIRECTION=±1 each)
  quit                  Send QUIT and exit
`

function main(): void {
	const { values } = parseArgs({
		options: {
			host: { type: 'string', default: CLI_DEFAULTS.host },
			port: { type: 'string', default: CLI_DEFAULTS.port },
			'key-size': { type: 'string', default: CLI_DEFAULTS.keySize },
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
	const keySize = Number(values['key-size'])
	const usageError =
		values.format !== 'rgb' && values.format !== 'png'
			? '--format must be rgb or png'
			: !Number.isInteger(port) || port < 1 || port > 65535
				? '--port must be an integer from 1 to 65535'
				: !Number.isInteger(keySize) || keySize < 1
					? '--key-size must be a positive integer'
					: null
	if (usageError) {
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
		bitmapFormat: values.format as 'rgb' | 'png',
		ping: !values['no-ping'],
	})
	const sink = new ImageSink({
		outDir,
		keySize,
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
	session.on('keyState', (params) => sink.handleKeyState(params))
	session.on('clear', () => sink.clear())
	session.on('fatal', () => process.exit(1))

	const rl = createInterface({ input: process.stdin, output: process.stdout, prompt: 'deck> ' })
	const registered = new Promise<void>((resolve) => session.once('registered', () => resolve()))
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
		// Let queued commands finish, but don't hang on EOF if the deck never registers.
		await Promise.race([pending, sleep(DEFAULT_OPTIONS.silenceTimeoutMs)])
		session.stop()
		setTimeout(() => process.exit(0), 300)
	})

	record(`## start ${JSON.stringify(session.opts)}`)
	session.start()
	rl.prompt()
}

if (import.meta.main) main()
