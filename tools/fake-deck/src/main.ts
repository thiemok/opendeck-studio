// CLI entry: run a fake deck against Companion, with a REPL for key and encoder input.
import { appendFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { parseArgs } from 'node:util'
import { ImageSink } from './images.ts'
import { CONTROL_IDS } from './protocol.ts'
import { DEFAULT_OPTIONS, FakeDeckSession } from './session.ts'

const HELP = `Usage: node src/main.ts [options]

Registers a fake deck (two LCD keys + one 128x64 encoder/OLED) with Companion over Satellite TCP.

Options:
  --host <host>         Companion host (default: 127.0.0.1)
  --port <port>         Satellite TCP port (default: ${DEFAULT_OPTIONS.port})
  --key-size <px>       Square key bitmap size (default: ${DEFAULT_OPTIONS.keySize})
  --format <rgb|png>    Requested bitmap format (default: rgb)
  --device-id <id>      Satellite DEVICEID (default: fake-deck-01)
  --out <dir>           Output directory for PNGs and protocol.log (default: out)
  --no-ping             Send no PING and disable the silence watchdog (measures the server idle timeout)
  -h, --help            Show this help

REPL commands:
  press <0|1>           Press and release LCD key 0 or 1
  push                  Press and release the encoder
  rotate <+n|-n>        Rotate the encoder by n steps
  quit                  Send QUIT and exit
`

/** Run one REPL command against a session. Returns false for `quit`. */
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
			session.rotate(CONTROL_IDS.encoder, steps)
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

/** Keep the protocol log readable: replace bitmap payloads with their size. */
const summarise = (line: string) =>
	line.replace(/BITMAP="([^"]*)"/, (_m, b64: string) => `BITMAP=<${b64.length} bytes${b64.startsWith('data:') ? ' data-url' : ''}>`)

function main(): void {
	const { values } = parseArgs({
		options: {
			host: { type: 'string', default: '127.0.0.1' },
			port: { type: 'string', default: String(DEFAULT_OPTIONS.port) },
			'key-size': { type: 'string', default: String(DEFAULT_OPTIONS.keySize) },
			format: { type: 'string', default: 'rgb' },
			'device-id': { type: 'string', default: 'fake-deck-01' },
			out: { type: 'string', default: 'out' },
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
	const record = (text: string) => appendFileSync(logFile, `${new Date().toISOString()} ${text}\n`)

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
	rl.on('line', async (input) => {
		if (!(await executeCommand(session, input))) rl.close()
		else rl.prompt()
	})
	rl.on('close', () => {
		session.stop()
		setTimeout(() => process.exit(0), 300)
	})

	record(`## start ${JSON.stringify(session.opts)}`)
	session.start()
	rl.prompt()
}

if (import.meta.main) main()
