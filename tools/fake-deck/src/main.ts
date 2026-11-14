import { appendFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { parseArgs } from 'node:util'
import { ImageSink } from './images.ts'
import { CONTROL_IDS } from './protocol.ts'
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

export async function executeCommand(session: FakeDeckSession, input: string): Promise<boolean> {
	const [cmd, arg] = input.trim().split(/\s+/)
	switch (cmd) {
		case 'press': {
			const key = arg === '0' ? CONTROL_IDS.key0 : arg === '1' ? CONTROL_IDS.key1 : null
			if (!key) {
				console.log('usage: press <0|1>')
				break
			}
			await session.press(key)
			break
		}
		case 'push':
			await session.press(CONTROL_IDS.encoder)
			break
		case 'rotate': {
			const steps = Number(arg)
			if (!Number.isInteger(steps) || steps === 0) {
				console.log('usage: rotate <+n|-n>')
				break
			}
			// One KEY-ROTATE per detent: Companion runs the rotate action once per message, whatever DIRECTION's magnitude.
			for (let i = 0; i < Math.abs(steps); i++) session.rotate(CONTROL_IDS.encoder, Math.sign(steps))
			break
		}
		case 'quit':
			return false
		case undefined:
		case '':
			break
		default:
			console.log(`unknown command: ${cmd} (press, push, rotate, quit)`)
	}
	return true
}

const summarise = (line: string) =>
	line.replace(/BITMAP="([^"]*)"/, (_m, b64: string) => `BITMAP=<${b64.length} bytes${b64.startsWith('data:') ? ' data-url' : ''}>`)

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
	if (values.format !== 'rgb' && values.format !== 'png') {
		console.error('--format must be rgb or png')
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

	const keySize = Number(values['key-size'])
	const session = new FakeDeckSession({
		...DEFAULT_OPTIONS,
		host: values.host,
		port: Number(values.port),
		deviceId: values['device-id'],
		keySize,
		bitmapFormat: values.format,
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

	session.on('line', (dir: 'in' | 'out', line: string) => record(`${dir === 'in' ? '<-' : '->'} ${summarise(line)}`))
	session.on('log', (l: string) => {
		record(`## ${l}`)
		console.log(`# ${l}`)
	})
	session.on('keyState', (params) => sink.handleKeyState(params))
	session.on('clear', () => sink.clear())
	session.on('fatal', () => process.exit(1))

	const rl = createInterface({ input: process.stdin, output: process.stdout, prompt: 'deck> ' })
	// readline does not await async handlers; chain commands so a press's release is sent before the next command.
	let pending = Promise.resolve()
	rl.on('line', (input) => {
		pending = pending.then(async () => {
			if (!(await executeCommand(session, input))) rl.close()
			else rl.prompt()
		})
	})
	rl.on('close', async () => {
		await pending
		session.stop()
		setTimeout(() => process.exit(0), 300)
	})

	record(`## start ${JSON.stringify(session.opts)}`)
	session.start()
	rl.prompt()
}

if (import.meta.main) main()
