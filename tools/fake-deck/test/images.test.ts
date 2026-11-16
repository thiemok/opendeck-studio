import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, existsSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, it } from 'node:test'
import { PNG } from 'pngjs'
import { ImageSink } from '../src/images.ts'
import { CONTROL_IDS } from '../src/protocol.ts'

const rgbOf = (w: number, h: number, fill: (x: number, y: number) => [number, number, number]) => {
	const buf = Buffer.alloc(w * h * 3)
	for (let y = 0; y < h; y++)
		for (let x = 0; x < w; x++) {
			const [r, g, b] = fill(x, y)
			buf.set([r, g, b], (y * w + x) * 3)
		}
	return buf
}

const readPng = (path: string) => PNG.sync.read(readFileSync(path))

const newSink = () => {
	const dir = mkdtempSync(join(tmpdir(), 'fake-deck-'))
	const logs: string[] = []
	return { dir, logs, sink: new ImageSink({ outDir: dir, keySize: 72, log: (l) => logs.push(l) }) }
}

describe('ImageSink', () => {
	it('writes a 72x72 PNG for a key from a raw rgb bitmap', () => {
		const { dir, sink } = newSink()
		const rgb = rgbOf(72, 72, () => [255, 0, 0])
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.key0, BITMAP: rgb.toString('base64') })
		const png = readPng(join(dir, '0-0.png'))
		assert.equal(png.width, 72)
		assert.equal(png.height, 72)
		assert.deepEqual([...png.data.subarray(0, 4)], [255, 0, 0, 255])
	})

	it('writes the 128x64 encoder PNG and a 1-bit preview', () => {
		const { dir, sink } = newSink()
		// left half dark grey, right half light grey
		const rgb = rgbOf(128, 64, (x) => (x < 64 ? [40, 40, 40] : [200, 200, 200]))
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.encoder, BITMAP: rgb.toString('base64') })
		const full = readPng(join(dir, '0-2.png'))
		assert.equal(full.width, 128)
		assert.equal(full.height, 64)
		const mono = readPng(join(dir, '0-2-mono.png'))
		assert.equal(mono.width, 128)
		assert.equal(mono.height, 64)
		const px = (x: number, y: number) => [...mono.data.subarray((y * 128 + x) * 4, (y * 128 + x) * 4 + 3)]
		assert.deepEqual(px(10, 10), [0, 0, 0])
		assert.deepEqual(px(100, 10), [255, 255, 255])
		for (const v of mono.data.filter((_, i) => i % 4 !== 3)) assert.ok(v === 0 || v === 255)
	})

	it('logs a size mismatch with expected and actual bytes and writes no PNG', () => {
		const { dir, sink, logs } = newSink()
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.key1, BITMAP: Buffer.alloc(100).toString('base64') })
		assert.equal(existsSync(join(dir, '0-1.png')), false)
		assert.ok(logs.some((l) => l.includes('15552') && l.includes('100')), logs.join('\n'))
	})

	it('decodes a data: PNG bitmap through the PNG path', () => {
		const { dir, sink } = newSink()
		const src = new PNG({ width: 72, height: 72 })
		src.data.fill(0)
		for (let i = 0; i < src.data.length; i += 4) src.data.set([0, 0, 255, 255], i)
		const dataUrl = `data:image/png;base64,${PNG.sync.write(src).toString('base64')}`
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.key0, BITMAP: dataUrl })
		const png = readPng(join(dir, '0-0.png'))
		assert.equal(png.width, 72)
		assert.deepEqual([...png.data.subarray(0, 4)], [0, 0, 255, 255])
	})

	it('logs raw and wire byte sizes, and the decoded encoder text', () => {
		const { sink, logs } = newSink()
		const rgb = rgbOf(128, 64, () => [0, 0, 0])
		sink.handleKeyState({
			CONTROLID: CONTROL_IDS.encoder,
			BITMAP: rgb.toString('base64'),
			TEXT: Buffer.from('Vol 42').toString('base64'),
		})
		assert.ok(logs.some((l) => l.includes('raw=24576') && l.includes('wire=32768')), logs.join('\n'))
		assert.ok(logs.some((l) => l.includes('Vol 42')), logs.join('\n'))
	})

	it('KEYS-CLEAR resets every control to black', () => {
		const { dir, sink } = newSink()
		sink.clear()
		for (const name of ['0-0', '0-1', '0-2']) {
			const png = readPng(join(dir, `${name}.png`))
			assert.deepEqual([...png.data.subarray(0, 3)], [0, 0, 0])
		}
	})

	it('rejects a PNG bitmap of the wrong size without writing it', () => {
		const { dir, sink, logs } = newSink()
		const src = new PNG({ width: 10, height: 10 })
		const dataUrl = `data:image/png;base64,${PNG.sync.write(src).toString('base64')}`
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.key0, BITMAP: dataUrl })
		assert.equal(existsSync(join(dir, '0-0.png')), false)
		assert.ok(logs.some((l) => l.includes('png is 10x10, expected 72x72')), logs.join('\n'))
	})

	it('rejects an unsupported data: bitmap format', () => {
		const { dir, sink, logs } = newSink()
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.key0, BITMAP: 'data:image/webp;base64,AAAA' })
		assert.equal(existsSync(join(dir, '0-0.png')), false)
		assert.ok(logs.some((l) => l.includes('unsupported bitmap format data:image/webp')), logs.join('\n'))
	})

	it('logs a corrupt PNG bitmap instead of throwing', () => {
		const { dir, sink, logs } = newSink()
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.key0, BITMAP: 'data:image/png;base64,bm90IGEgcG5n' })
		assert.equal(existsSync(join(dir, '0-0.png')), false)
		assert.ok(logs.some((l) => l.includes('png decode failed')), logs.join('\n'))
	})

	it('logs a failed file write instead of throwing', () => {
		const { dir, sink, logs } = newSink()
		rmSync(dir, { recursive: true })
		writeFileSync(dir, 'not a directory')
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.key0, BITMAP: rgbOf(72, 72, () => [1, 2, 3]).toString('base64') })
		assert.ok(logs.some((l) => l.includes('write failed')), logs.join('\n'))
	})

	it('thresholds the 1-bit preview at luma 128', () => {
		const { dir, sink } = newSink()
		const rgb = rgbOf(128, 64, (x) => (x < 64 ? [127, 127, 127] : [128, 128, 128]))
		sink.handleKeyState({ CONTROLID: CONTROL_IDS.encoder, BITMAP: rgb.toString('base64') })
		const mono = readPng(join(dir, '0-2-mono.png'))
		const px = (x: number) => [...mono.data.subarray(x * 4, x * 4 + 3)]
		assert.deepEqual(px(10), [0, 0, 0])
		assert.deepEqual(px(100), [255, 255, 255])
	})

	it('on a bench surface sizes every control as a key and saves what came off the wire', () => {
		const dir = mkdtempSync(join(tmpdir(), 'fake-deck-'))
		const logs: string[] = []
		const sink = new ImageSink({ outDir: dir, keySize: 8, keys: 16, saveRaw: true, quiet: true, log: (l) => logs.push(l) })
		const png = new PNG({ width: 8, height: 8 })
		png.data.fill(200)
		const wire = PNG.sync.write(png)
		const result = sink.handleKeyState({ CONTROLID: '0/2', BITMAP: `data:image/png;base64,${wire.toString('base64')}` })
		assert.equal(result.ok, true)
		assert.deepEqual(readFileSync(join(dir, '0-2.wire.png')), wire)
		assert.equal(readPng(join(dir, '0-2.png')).width, 8, '0/2 is a key here, not the encoder')
		assert.equal(existsSync(join(dir, '0-2-mono.png')), false)

		const rgb = rgbOf(8, 8, () => [1, 2, 3])
		assert.equal(sink.handleKeyState({ CONTROLID: '1/7', BITMAP: rgb.toString('base64') }).ok, true)
		assert.deepEqual(readFileSync(join(dir, '1-7.rgb')), rgb)
		assert.deepEqual(logs, [], 'quiet: no line per image')
		rmSync(dir, { recursive: true })
	})
})
