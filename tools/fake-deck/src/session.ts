import { EventEmitter } from 'node:events'
import net from 'node:net'
import { setTimeout as sleep } from 'node:timers/promises'
import {
	buildLayoutManifest,
	formatLine,
	parseCaps,
	parseLine,
	type LineArgs,
	type Message,
	type Params,
} from './protocol.ts'

export interface SessionOptions {
	host: string
	port: number
	deviceId: string
	keySize: number
	bitmapFormat: 'rgb' | 'png'
	ping: boolean
	pingIntervalMs: number
	silenceTimeoutMs: number
	backoffInitialMs: number
	backoffMaxMs: number
	holdMs: number
}

interface SessionEvents {
	log: [line: string]
	line: [direction: 'in' | 'out', line: string]
	keyState: [params: Params]
	clear: []
	fatal: [message: string]
}

/**
 * CAPS only exists from Satellite API 1.10, so anything older can never advertise NONSQUARE (API 1.11).
 * A 1.10 server still reaches the CAPS check, which rejects it there. An unparsable version counts as too old.
 */
function hasCaps(apiVersion: string): boolean {
	const [major, minor] = apiVersion.split('.').map(Number)
	return major > 1 || (major === 1 && minor >= 10)
}

export const DEFAULT_OPTIONS: Omit<SessionOptions, 'host' | 'deviceId'> = {
	port: 16622,
	keySize: 72,
	bitmapFormat: 'rgb',
	ping: true,
	pingIntervalMs: 2000,
	silenceTimeoutMs: 5000,
	backoffInitialMs: 1000,
	backoffMaxMs: 10000,
	holdMs: 100,
}

export type SessionState = 'idle' | 'connecting' | 'handshake' | 'registered' | 'backoff' | 'stopped'

export class FakeDeckSession extends EventEmitter<SessionEvents> {
	readonly opts: SessionOptions
	state: SessionState = 'idle'
	#socket: net.Socket | null = null
	#buffer = ''
	#attempt = 0
	#pingTimer: NodeJS.Timeout | null = null
	#silenceTimer: NodeJS.Timeout | null = null
	#retryTimer: NodeJS.Timeout | null = null

	constructor(opts: SessionOptions) {
		super()
		this.opts = opts
	}

	start(): void {
		if (this.state !== 'idle' && this.state !== 'backoff') return
		this.#clearTimers()
		this.#connect()
	}

	stop(): void {
		if (this.state === 'stopped') return
		const socket = this.#socket
		this.#setState('stopped')
		this.#clearTimers()
		if (socket && !socket.destroyed) {
			socket.end(formatLine('QUIT'))
			setTimeout(() => socket.destroy(), 200).unref()
		}
		this.#socket = null
	}

	async press(controlId: string): Promise<void> {
		const control = this.#controlArgs(controlId)
		this.#send('KEY-PRESS', { ...control, PRESSED: true })
		await sleep(this.opts.holdMs)
		this.#send('KEY-PRESS', { ...control, PRESSED: false })
	}

	rotate(controlId: string, direction: number): void {
		this.#send('KEY-ROTATE', { ...this.#controlArgs(controlId), DIRECTION: direction })
	}

	#controlArgs(controlId: string): LineArgs {
		return { DEVICEID: this.opts.deviceId, CONTROLID: controlId }
	}

	#setState(state: SessionState): void {
		if (this.state === state) return
		this.state = state
		this.emit('log', `state: ${state}`)
	}

	#connect(): void {
		this.#setState('connecting')
		this.#buffer = ''
		const socket = net.connect({ host: this.opts.host, port: this.opts.port })
		this.#socket = socket
		socket.setNoDelay(true)
		socket.setEncoding('utf8')
		// A host that drops SYNs would otherwise hold this attempt for the OS connect timeout (~2 min).
		socket.setTimeout(this.opts.silenceTimeoutMs, () => {
			this.emit('log', `connect timed out after ${this.opts.silenceTimeoutMs} ms`)
			socket.destroy()
		})

		socket.on('connect', () => {
			if (this.#socket !== socket) return
			socket.setTimeout(0)
			this.#setState('handshake')
			this.#armSilence()
		})
		socket.on('data', (chunk: string) => {
			if (this.#socket !== socket) return
			this.#armSilence()
			this.#buffer += chunk
			let i
			while ((i = this.#buffer.indexOf('\n')) !== -1) {
				const raw = this.#buffer.slice(0, i).replace(/\r$/, '')
				this.#buffer = this.#buffer.slice(i + 1)
				if (raw.trim() === '') continue
				this.emit('line', 'in', raw)
				this.#handle(parseLine(raw))
			}
		})
		socket.on('error', (err) => {
			if (this.#socket === socket) this.emit('log', `socket error: ${err.message}`)
		})
		socket.on('close', () => {
			if (this.#socket !== socket) return
			this.#socket = null
			this.#clearTimers()
			if (this.state === 'stopped') return
			this.emit('log', 'disconnected')
			this.#scheduleReconnect()
		})
	}

	#handle(msg: Message): void {
		switch (msg.command) {
			case 'BEGIN': {
				const api = String(msg.params.ApiVersion ?? '')
				this.emit('log', `Companion ${msg.params.CompanionVersion ?? '?'} ApiVersion ${api}`)
				if (!hasCaps(api)) this.#fatal(`ApiVersion ${api} has no NONSQUARE; needs a current Companion`)
				break
			}
			case 'CAPS': {
				const caps = parseCaps(msg.params)
				this.emit('log', `caps: ${JSON.stringify(caps)}`)
				if (!caps.nonSquare) return this.#fatal('CAPS lacks NONSQUARE; needs a current Companion')
				this.#register(caps.bitmapFormats)
				break
			}
			case 'ADD-DEVICE':
				if (msg.status === 'OK') {
					this.#attempt = 0
					this.#setState('registered')
					this.#startPing()
				} else {
					this.emit('log', `ADD-DEVICE failed: ${msg.params.MESSAGE ?? '(no message)'}`)
					this.#socket?.destroy()
				}
				break
			case 'PING':
				this.#sendRaw(`PONG ${msg.body}\n`)
				break
			case 'KEY-STATE':
				this.emit('keyState', msg.params)
				break
			case 'KEYS-CLEAR':
				this.emit('clear')
				break
			case 'ERROR':
				this.emit('log', `server error: ${msg.params.MESSAGE ?? ''}`)
				break
		}
	}

	#register(serverFormats: string[]): void {
		const args: Record<string, string> = {
			DEVICEID: this.opts.deviceId,
			SERIAL: `opendeck:${this.opts.deviceId}`,
			PRODUCT_NAME: 'opendeck fake deck',
			LAYOUT_MANIFEST: buildLayoutManifest({ keySize: this.opts.keySize }),
		}
		if (this.opts.bitmapFormat === 'png') {
			if (serverFormats.includes('png')) args.BITMAP_FORMAT = 'png'
			else this.emit('log', 'png requested but not advertised; using rgb')
		}
		this.#send('ADD-DEVICE', args)
	}

	#fatal(message: string): void {
		this.emit('log', `fatal: ${message}`)
		this.stop()
		this.emit('fatal', message)
	}

	#send(command: string, args: LineArgs): void {
		this.#sendRaw(formatLine(command, args))
	}

	#sendRaw(line: string): void {
		const socket = this.#socket
		if (!socket || socket.destroyed || !socket.writable) {
			this.emit('log', `not connected; dropped: ${line.trimEnd()}`)
			return
		}
		this.emit('line', 'out', line.trimEnd())
		socket.write(line)
	}

	#startPing(): void {
		if (!this.opts.ping) return
		if (this.#pingTimer) clearInterval(this.#pingTimer)
		this.#pingTimer = setInterval(() => this.#sendRaw(`PING ${Date.now()}\n`), this.opts.pingIntervalMs)
	}

	#armSilence(): void {
		if (!this.opts.ping) return
		if (this.#silenceTimer) clearTimeout(this.#silenceTimer)
		this.#silenceTimer = setTimeout(() => {
			this.emit('log', `no data for ${this.opts.silenceTimeoutMs} ms; dropping link`)
			this.#socket?.destroy()
		}, this.opts.silenceTimeoutMs)
	}

	#scheduleReconnect(): void {
		const delay = Math.min(this.opts.backoffInitialMs * 2 ** this.#attempt, this.opts.backoffMaxMs)
		this.#attempt++
		this.#setState('backoff')
		this.emit('log', `reconnecting in ${delay} ms`)
		this.#retryTimer = setTimeout(() => this.#connect(), delay)
	}

	#clearTimers(): void {
		for (const t of [this.#pingTimer, this.#silenceTimer, this.#retryTimer]) if (t) clearTimeout(t)
		this.#pingTimer = this.#silenceTimer = this.#retryTimer = null
	}
}
