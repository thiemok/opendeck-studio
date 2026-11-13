import assert from 'node:assert/strict'
import net from 'node:net'
import { after, describe, it } from 'node:test'
import { executeCommand } from '../src/main.ts'
import { CONTROL_IDS, parseLine, type Message } from '../src/protocol.ts'
import { FakeDeckSession, type SessionOptions } from '../src/session.ts'

const HANDSHAKE = 'BEGIN CompanionVersion="5.1.3" ApiVersion="1.14.0" \nCAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1 \n'

/** Minimal scripted Companion: records every client line per connection and lets the test reply. */
class FakeCompanion {
	server = net.createServer()
	connections: { socket: net.Socket; lines: Message[] }[] = []
	onConnect: (socket: net.Socket) => void = (s) => s.write(HANDSHAKE)
	onLine: (msg: Message, socket: net.Socket) => void = (msg, socket) => {
		if (msg.command === 'ADD-DEVICE') socket.write(`ADD-DEVICE OK DEVICEID="${msg.params.DEVICEID}" \n`)
		if (msg.command === 'PING') socket.write(`PONG ${msg.body} \n`)
	}

	async listen(): Promise<number> {
		this.server.on('connection', (socket) => {
			const conn = { socket, lines: [] as Message[] }
			this.connections.push(conn)
			let buf = ''
			socket.on('data', (d) => {
				buf += d
				let i
				while ((i = buf.indexOf('\n')) !== -1) {
					const msg = parseLine(buf.slice(0, i))
					buf = buf.slice(i + 1)
					conn.lines.push(msg)
					this.onLine(msg, socket)
				}
			})
			socket.on('error', () => {})
			this.onConnect(socket)
		})
		await new Promise<void>((r) => this.server.listen(0, '127.0.0.1', r))
		return (this.server.address() as net.AddressInfo).port
	}

	sent(command: string, conn = this.connections.length - 1): Message[] {
		return (this.connections[conn]?.lines ?? []).filter((m) => m.command === command)
	}

	close(): void {
		for (const c of this.connections) c.socket.destroy()
		this.server.close()
	}
}

const sessions: FakeDeckSession[] = []
const servers: FakeCompanion[] = []
after(() => {
	for (const s of sessions) s.stop()
	for (const s of servers) s.close()
})

async function setup(opts: Partial<SessionOptions> = {}, configure?: (c: FakeCompanion) => void) {
	const companion = new FakeCompanion()
	configure?.(companion)
	servers.push(companion)
	const port = await companion.listen()
	const session = new FakeDeckSession({
		host: '127.0.0.1',
		port,
		deviceId: 'fake-test',
		keySize: 72,
		bitmapFormat: 'rgb',
		ping: true,
		pingIntervalMs: 40,
		silenceTimeoutMs: 200,
		backoffInitialMs: 30,
		backoffMaxMs: 100,
		holdMs: 5,
		...opts,
	})
	sessions.push(session)
	const logs: string[] = []
	session.on('log', (l: string) => logs.push(l))
	return { companion, session, logs }
}

const waitFor = async (cond: () => boolean, timeoutMs = 2000): Promise<void> => {
	const start = Date.now()
	while (!cond()) {
		if (Date.now() - start > timeoutMs) throw new Error('timed out waiting for condition')
		await new Promise((r) => setTimeout(r, 5))
	}
}

describe('FakeDeckSession', () => {
	it('registers once with SERIAL and a three-control layout manifest after the handshake', async () => {
		const { companion, session } = await setup()
		session.start()
		await waitFor(() => session.state === 'registered')
		const adds = companion.sent('ADD-DEVICE')
		assert.equal(adds.length, 1)
		assert.equal(adds[0].params.DEVICEID, 'fake-test')
		assert.equal(adds[0].params.SERIAL, 'opendeck:fake-test')
		const manifest = JSON.parse(Buffer.from(String(adds[0].params.LAYOUT_MANIFEST), 'base64').toString())
		assert.equal(Object.keys(manifest.controls).length, 3)
		assert.equal(adds[0].params.BITMAP_FORMAT, undefined)
		session.stop()
	})

	it('refuses to register and reports a fatal error when CAPS lacks NONSQUARE', async () => {
		const { companion, session } = await setup({}, (c) => {
			c.onConnect = (s) => s.write('BEGIN CompanionVersion="4.2.0" ApiVersion="1.9.0" \n')
		})
		const fatal = new Promise<string>((r) => session.once('fatal', r))
		session.start()
		await new Promise((r) => setTimeout(r, 20))
		// API 1.9 sends no CAPS at all; the client must give up rather than wait forever
		const message = await fatal
		assert.match(message, /NONSQUARE|current Companion/)
		assert.equal(session.state, 'stopped')
		assert.equal(companion.sent('ADD-DEVICE').length, 0)
	})

	it('requests png only when the flag is set and CAPS advertises it', async () => {
		const withPng = await setup({ bitmapFormat: 'png' })
		withPng.session.start()
		await waitFor(() => withPng.session.state === 'registered')
		assert.equal(withPng.companion.sent('ADD-DEVICE')[0].params.BITMAP_FORMAT, 'png')
		withPng.session.stop()

		const noPng = await setup({ bitmapFormat: 'png' }, (c) => {
			c.onConnect = (s) => s.write('BEGIN ApiVersion="1.14.0" \nCAPS NONSQUARE=1 BITMAP_FORMATS="rgb" \n')
		})
		noPng.session.start()
		await waitFor(() => noPng.session.state === 'registered')
		assert.equal(noPng.companion.sent('ADD-DEVICE')[0].params.BITMAP_FORMAT, undefined)
		noPng.session.stop()
	})

	it('answers server PING with PONG carrying the same payload', async () => {
		const { companion, session } = await setup()
		session.start()
		await waitFor(() => session.state === 'registered')
		companion.connections[0].socket.write('PING abc \n')
		await waitFor(() => companion.sent('PONG').length > 0)
		assert.equal(companion.sent('PONG')[0].body, 'abc')
		session.stop()
	})

	it('sends PING repeatedly while registered', async () => {
		const { companion, session } = await setup()
		session.start()
		await waitFor(() => companion.sent('PING').length >= 3)
		session.stop()
	})

	it('with ping disabled sends no PING and survives server silence until the server closes', async () => {
		const { companion, session } = await setup({ ping: false })
		session.start()
		await waitFor(() => session.state === 'registered')
		await new Promise((r) => setTimeout(r, 400)) // twice the silence timeout
		assert.equal(companion.sent('PING').length, 0)
		assert.equal(session.state, 'registered')
		assert.equal(companion.connections.length, 1)
		companion.connections[0].socket.destroy()
		await waitFor(() => session.state !== 'registered')
		session.stop()
	})

	it('Covers AE1: reconnects and re-registers after the server closes the socket', async () => {
		const { companion, session, logs } = await setup()
		session.start()
		await waitFor(() => session.state === 'registered')
		companion.connections[0].socket.destroy()
		await waitFor(() => logs.some((l) => /disconnected/i.test(l)))
		await waitFor(() => companion.connections.length === 2 && companion.sent('ADD-DEVICE', 1).length === 1)
		await waitFor(() => session.state === 'registered')
		session.stop()
	})

	it('Covers AE1: declares a silent link dead within the silence timeout and reconnects', async () => {
		const { companion, session } = await setup({}, (c) => {
			const base = c.onLine
			// Stop answering PING on the first connection only
			c.onLine = (msg, socket) => {
				if (msg.command === 'PING' && socket === c.connections[0].socket) return
				base(msg, socket)
			}
		})
		session.start()
		await waitFor(() => session.state === 'registered')
		const registeredAt = Date.now()
		await waitFor(() => companion.connections.length === 2, 1000)
		assert.ok(Date.now() - registeredAt < 600, 'silence detection took too long')
		await waitFor(() => companion.sent('ADD-DEVICE', 1).length === 1)
		session.stop()
	})

	it('logs ADD-DEVICE ERROR and backs off instead of reconnecting without delay', async () => {
		const { companion, session, logs } = await setup({ backoffInitialMs: 150, backoffMaxMs: 150 }, (c) => {
			c.onLine = (msg, socket) => {
				if (msg.command === 'ADD-DEVICE') socket.write('ADD-DEVICE ERROR DEVICEID="fake-test" MESSAGE="Device exists elsewhere" \n')
			}
		})
		session.start()
		await waitFor(() => logs.some((l) => l.includes('Device exists elsewhere')))
		await new Promise((r) => setTimeout(r, 100))
		assert.equal(companion.connections.length, 1, 'reconnected before the backoff elapsed')
		await waitFor(() => companion.connections.length === 2, 1000)
		session.stop()
	})

	it('emits KEY-STATE and KEYS-CLEAR to listeners', async () => {
		const { companion, session } = await setup()
		const states: Record<string, unknown>[] = []
		let cleared = 0
		session.on('keyState', (p) => states.push(p))
		session.on('clear', () => cleared++)
		session.start()
		await waitFor(() => session.state === 'registered')
		companion.connections[0].socket.write('KEY-STATE DEVICEID="fake-test" CONTROLID="0/1" COLOR="#ff0000" \nKEYS-CLEAR DEVICEID="fake-test" \n')
		await waitFor(() => states.length === 1 && cleared === 1)
		assert.equal(states[0].CONTROLID, '0/1')
		session.stop()
	})
})

describe('REPL commands', () => {
	it('press <key> sends KEY-PRESS pressed then released on that key', async () => {
		const { companion, session } = await setup()
		session.start()
		await waitFor(() => session.state === 'registered')
		await executeCommand(session, 'press 1')
		await waitFor(() => companion.sent('KEY-PRESS').length === 2)
		const [down, up] = companion.sent('KEY-PRESS')
		assert.equal(down.params.CONTROLID, CONTROL_IDS.key1)
		assert.equal(down.params.PRESSED, '1')
		assert.equal(up.params.CONTROLID, CONTROL_IDS.key1)
		assert.equal(up.params.PRESSED, '0')
		session.stop()
	})

	it('rotate +1/-1 and push act on the encoder control', async () => {
		const { companion, session } = await setup()
		session.start()
		await waitFor(() => session.state === 'registered')
		await executeCommand(session, 'rotate +1')
		await executeCommand(session, 'rotate -1')
		await executeCommand(session, 'push')
		await waitFor(() => companion.sent('KEY-ROTATE').length === 2 && companion.sent('KEY-PRESS').length === 2)
		const rotates = companion.sent('KEY-ROTATE')
		assert.deepEqual(
			rotates.map((m) => [m.params.CONTROLID, m.params.DIRECTION]),
			[
				[CONTROL_IDS.encoder, '1'],
				[CONTROL_IDS.encoder, '-1'],
			]
		)
		assert.deepEqual(
			companion.sent('KEY-PRESS').map((m) => [m.params.CONTROLID, m.params.PRESSED]),
			[
				[CONTROL_IDS.encoder, '1'],
				[CONTROL_IDS.encoder, '0'],
			]
		)
		session.stop()
	})
})
