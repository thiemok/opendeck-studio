import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { PNG } from 'pngjs'
import { benchControlIds, CONTROL_IDS, ENCODER_SIZE, type Params } from './protocol.ts'

export interface ImageSinkOptions {
	outDir: string
	keySize: number
	/** A bench surface (THI-76): this many controls, all keys. 0 or absent: two keys and the encoder. */
	keys?: number
	/** Also write each image as it came off the wire: `<id>.wire.png` for png, `<id>.rgb` for rgb. */
	saveRaw?: boolean
	/** Don't log a line per image (a 32-key page change would print 32). */
	quiet?: boolean
	log: (line: string) => void
}

/** What handling one image cost, for page-change timing. */
export interface ImageResult {
	ok: boolean
	decodeMs: number
}

interface Size {
	w: number
	h: number
}

const MONO_THRESHOLD = 128

const fileName = (controlId: string) => controlId.replace(/[^a-zA-Z0-9-]/g, '-')

function encodePng(rgb: Buffer, { w, h }: Size): Buffer {
	const png = new PNG({ width: w, height: h })
	for (let i = 0, j = 0; i < rgb.length; i += 3, j += 4) {
		png.data[j] = rgb[i]
		png.data[j + 1] = rgb[i + 1]
		png.data[j + 2] = rgb[i + 2]
		png.data[j + 3] = 255
	}
	return PNG.sync.write(png)
}

function toMono(rgb: Buffer): Buffer {
	const out = Buffer.alloc(rgb.length)
	for (let i = 0; i < rgb.length; i += 3) {
		const luma = (299 * rgb[i] + 587 * rgb[i + 1] + 114 * rgb[i + 2]) / 1000
		out[i] = out[i + 1] = out[i + 2] = luma >= MONO_THRESHOLD ? 255 : 0
	}
	return out
}

type Decoded = { ok: true; rgb: Buffer; format: 'rgb' | 'png'; wire: Buffer } | { ok: false; reason: string }

function decodeBitmap(bitmap: string, size: Size): Decoded {
	const expected = size.w * size.h * 3
	if (bitmap.startsWith('data:image/png;base64,')) {
		let png: PNG
		const wire = Buffer.from(bitmap.slice(bitmap.indexOf(',') + 1), 'base64')
		try {
			png = PNG.sync.read(wire)
		} catch (err) {
			return { ok: false, reason: `png decode failed: ${(err as Error).message}` }
		}
		if (png.width !== size.w || png.height !== size.h) {
			return { ok: false, reason: `png is ${png.width}x${png.height}, expected ${size.w}x${size.h}` }
		}
		const rgb = Buffer.alloc(expected)
		for (let i = 0, j = 0; j < png.data.length; i += 3, j += 4) {
			rgb[i] = png.data[j]
			rgb[i + 1] = png.data[j + 1]
			rgb[i + 2] = png.data[j + 2]
		}
		return { ok: true, rgb, format: 'png', wire }
	}
	if (bitmap.startsWith('data:')) {
		return { ok: false, reason: `unsupported bitmap format ${bitmap.slice(0, bitmap.indexOf(';'))}` }
	}
	const rgb = Buffer.from(bitmap, 'base64')
	if (rgb.length !== expected) {
		return { ok: false, reason: `size mismatch: expected ${expected} bytes, got ${rgb.length}` }
	}
	return { ok: true, rgb, format: 'rgb', wire: rgb }
}

export class ImageSink {
	readonly #opts: ImageSinkOptions

	constructor(opts: ImageSinkOptions) {
		this.#opts = opts
		mkdirSync(opts.outDir, { recursive: true })
	}

	#sizeOf(controlId: string): Size {
		const isEncoder = !this.#opts.keys && controlId === CONTROL_IDS.encoder
		return isEncoder ? ENCODER_SIZE : { w: this.#opts.keySize, h: this.#opts.keySize }
	}

	#controlIds(): string[] {
		return this.#opts.keys ? benchControlIds(this.#opts.keys) : Object.values(CONTROL_IDS)
	}

	#write(controlId: string, rgb: Buffer): void {
		const size = this.#sizeOf(controlId)
		const name = fileName(controlId)
		try {
			writeFileSync(join(this.#opts.outDir, `${name}.png`), encodePng(rgb, size))
			if (!this.#opts.keys && controlId === CONTROL_IDS.encoder) {
				writeFileSync(join(this.#opts.outDir, `${name}-mono.png`), encodePng(toMono(rgb), size))
			}
		} catch (err) {
			this.#opts.log(`[${controlId}] write failed: ${(err as Error).message}`)
		}
	}

	handleKeyState(params: Params): ImageResult {
		const controlId = String(params.CONTROLID ?? params.KEY ?? '?')
		const { log } = this.#opts

		if (typeof params.TEXT === 'string') {
			log(`[${controlId}] text=${JSON.stringify(Buffer.from(params.TEXT, 'base64').toString())}`)
		}
		if (typeof params.BITMAP !== 'string') return { ok: false, decodeMs: 0 }

		const size = this.#sizeOf(controlId)
		const started = performance.now()
		const decoded = decodeBitmap(params.BITMAP, size)
		const decodeMs = Math.round(performance.now() - started)
		if (!decoded.ok) {
			log(`[${controlId}] bitmap rejected: ${decoded.reason}`)
			return { ok: false, decodeMs }
		}
		if (!this.#opts.quiet) {
			log(
				`[${controlId}] bitmap ${decoded.format} ${size.w}x${size.h} raw=${decoded.rgb.length} wire=${params.BITMAP.length}`
			)
		}
		this.#write(controlId, decoded.rgb)
		if (this.#opts.saveRaw) {
			const name = `${fileName(controlId)}.${decoded.format === 'png' ? 'wire.png' : 'rgb'}`
			try {
				writeFileSync(join(this.#opts.outDir, name), decoded.wire)
			} catch (err) {
				log(`[${controlId}] raw write failed: ${(err as Error).message}`)
			}
		}
		return { ok: true, decodeMs }
	}

	clear(): void {
		for (const controlId of this.#controlIds()) {
			const { w, h } = this.#sizeOf(controlId)
			this.#write(controlId, Buffer.alloc(w * h * 3))
		}
	}
}
