import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { PNG } from 'pngjs'
import { CONTROL_IDS, ENCODER_SIZE, type Params } from './protocol.ts'

export interface ImageSinkOptions {
	outDir: string
	keySize: number
	log: (line: string) => void
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
		const luma = 0.299 * rgb[i] + 0.587 * rgb[i + 1] + 0.114 * rgb[i + 2]
		out.fill(luma >= MONO_THRESHOLD ? 255 : 0, i, i + 3)
	}
	return out
}

type Decoded = { ok: true; rgb: Buffer; format: string } | { ok: false; reason: string }

function decodeBitmap(bitmap: string, size: Size): Decoded {
	const expected = size.w * size.h * 3
	if (bitmap.startsWith('data:image/png;base64,')) {
		const png = PNG.sync.read(Buffer.from(bitmap.slice(bitmap.indexOf(',') + 1), 'base64'))
		if (png.width !== size.w || png.height !== size.h) {
			return { ok: false, reason: `png is ${png.width}x${png.height}, expected ${size.w}x${size.h}` }
		}
		const rgb = Buffer.alloc(expected)
		for (let i = 0, j = 0; j < png.data.length; i += 3, j += 4) {
			rgb[i] = png.data[j]
			rgb[i + 1] = png.data[j + 1]
			rgb[i + 2] = png.data[j + 2]
		}
		return { ok: true, rgb, format: 'png' }
	}
	if (bitmap.startsWith('data:')) {
		return { ok: false, reason: `unsupported bitmap format ${bitmap.slice(0, bitmap.indexOf(';'))}` }
	}
	const rgb = Buffer.from(bitmap, 'base64')
	if (rgb.length !== expected) {
		return { ok: false, reason: `size mismatch: expected ${expected} bytes, got ${rgb.length}` }
	}
	return { ok: true, rgb, format: 'rgb' }
}

export class ImageSink {
	readonly #opts: ImageSinkOptions

	constructor(opts: ImageSinkOptions) {
		this.#opts = opts
		mkdirSync(opts.outDir, { recursive: true })
	}

	#sizeOf(controlId: string): Size {
		return controlId === CONTROL_IDS.encoder ? ENCODER_SIZE : { w: this.#opts.keySize, h: this.#opts.keySize }
	}

	#write(controlId: string, rgb: Buffer): void {
		const size = this.#sizeOf(controlId)
		const name = fileName(controlId)
		writeFileSync(join(this.#opts.outDir, `${name}.png`), encodePng(rgb, size))
		if (controlId === CONTROL_IDS.encoder) {
			writeFileSync(join(this.#opts.outDir, `${name}-mono.png`), encodePng(toMono(rgb), size))
		}
	}

	handleKeyState(params: Params): void {
		const controlId = String(params.CONTROLID ?? params.KEY ?? '?')
		const { log } = this.#opts

		if (typeof params.TEXT === 'string') {
			log(`[${controlId}] text=${JSON.stringify(Buffer.from(params.TEXT, 'base64').toString())}`)
		}
		if (typeof params.BITMAP !== 'string') return

		const size = this.#sizeOf(controlId)
		const decoded = decodeBitmap(params.BITMAP, size)
		if (!decoded.ok) {
			log(`[${controlId}] bitmap rejected: ${decoded.reason}`)
			return
		}
		log(
			`[${controlId}] bitmap ${decoded.format} ${size.w}x${size.h} raw=${decoded.rgb.length} wire=${params.BITMAP.length}`
		)
		this.#write(controlId, decoded.rgb)
	}

	clear(): void {
		for (const controlId of Object.values(CONTROL_IDS)) {
			const { w, h } = this.#sizeOf(controlId)
			this.#write(controlId, Buffer.alloc(w * h * 3))
		}
	}
}
