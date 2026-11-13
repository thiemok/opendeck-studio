export type ParamValue = string | true
export type Params = Record<string, ParamValue>
export type LineArgs = Record<string, string | number | boolean>

export interface Message {
	command: string
	status: 'OK' | 'ERROR' | null
	params: Params
	/** Everything after the command word, unparsed (PING/PONG payloads). */
	body: string
}

export interface Caps {
	subscriptions: boolean
	nonSquare: boolean
	rotaryAmount: boolean
	bitmapFormats: string[]
}

export const CONTROL_IDS = {
	key0: '0/0',
	key1: '0/1',
	encoder: '0/2',
} as const

export const ENCODER_SIZE = { w: 128, h: 64 } as const

const escapeValue = (value: string): string => value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')

export function formatLine(command: string, args: LineArgs = {}): string {
	const chunks = [command]
	for (const [key, value] of Object.entries(args)) {
		if (typeof value === 'boolean') chunks.push(`${key}=${value ? 1 : 0}`)
		else if (typeof value === 'number') chunks.push(`${key}=${value}`)
		else chunks.push(`${key}="${escapeValue(value)}"`)
	}
	return `${chunks.join(' ')}\n`
}

/**
 * Split `KEY=value KEY="quoted value" FLAG` into params. Mirrors Companion's own parser:
 * a backslash escapes the next character, quotes toggle space-splitting, and only the first `=`
 * separates key from value (base64 padding keeps its `=`).
 */
export function parseParams(body: string): Params {
	const fragments = ['']
	let quoted = false
	for (let i = 0; i < body.length; i++) {
		const c = body[i]
		if (c === '\\') {
			fragments[fragments.length - 1] += body[i + 1] ?? ''
			i++
		} else if (c === '"') {
			quoted = !quoted
		} else if (c === ' ' && !quoted) {
			fragments.push('')
		} else {
			fragments[fragments.length - 1] += c
		}
	}

	const params: Params = {}
	for (const fragment of fragments) {
		if (fragment === '') continue
		const eq = fragment.indexOf('=')
		if (eq === -1) params[fragment] = true
		else if (eq > 0) params[fragment.slice(0, eq)] = fragment.slice(eq + 1)
	}
	return params
}

export function parseLine(line: string): Message {
	const trimmed = line.replace(/\r?\n$/, '')
	const space = trimmed.indexOf(' ')
	const command = (space === -1 ? trimmed : trimmed.slice(0, space)).toUpperCase()
	let body = space === -1 ? '' : trimmed.slice(space + 1)
	const rawBody = body.trimEnd()

	let status: Message['status'] = null
	const statusMatch = /^(OK|ERROR)(?:\s|$)/.exec(body)
	if (statusMatch && command !== 'ERROR') {
		status = statusMatch[1] as 'OK' | 'ERROR'
		body = body.slice(statusMatch[0].length)
	}

	return { command, status, params: parseParams(body), body: rawBody }
}

export function toBool(value: ParamValue | undefined): boolean {
	if (value === true) return true
	if (value === undefined) return false
	const v = value.toLowerCase()
	return v === '1' || v === 'true'
}

export function parseCaps(params: Params): Caps {
	const formats = typeof params.BITMAP_FORMATS === 'string' ? params.BITMAP_FORMATS.split(',').filter(Boolean) : []
	return {
		subscriptions: toBool(params.SUBSCRIPTIONS),
		nonSquare: toBool(params.NONSQUARE),
		rotaryAmount: toBool(params.ROTARY_AMOUNT),
		bitmapFormats: formats.length ? formats : ['rgb'],
	}
}

export function buildLayoutManifest(opts: { keySize: number }): string {
	const manifest = {
		stylePresets: {
			default: {},
			key: { bitmap: { w: opts.keySize, h: opts.keySize } },
			encoder: { bitmap: { ...ENCODER_SIZE }, text: true, textStyle: true },
		},
		controls: {
			[CONTROL_IDS.key0]: { row: 0, column: 0, stylePreset: 'key' },
			[CONTROL_IDS.key1]: { row: 0, column: 1, stylePreset: 'key' },
			[CONTROL_IDS.encoder]: { row: 0, column: 2, stylePreset: 'encoder' },
		},
	}
	return Buffer.from(JSON.stringify(manifest)).toString('base64')
}
