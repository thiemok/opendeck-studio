import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
	buildLayoutManifest,
	CONTROL_IDS,
	formatLine,
	parseCaps,
	parseLine,
	summariseLine,
	toBool,
} from '../src/protocol.ts'

describe('formatLine', () => {
	it('quotes string values, maps booleans to 0/1 and ends with a newline', () => {
		const line = formatLine('KEY-PRESS', { DEVICEID: 'fake-01', CONTROLID: '0/1', PRESSED: true })
		assert.equal(line, 'KEY-PRESS DEVICEID="fake-01" CONTROLID="0/1" PRESSED=1\n')
	})

	it('writes numbers bare, including negative rotation steps', () => {
		assert.equal(formatLine('KEY-ROTATE', { DIRECTION: -1 }), 'KEY-ROTATE DIRECTION=-1\n')
	})

	it('escapes quotes and backslashes inside values so Companion parses them back', () => {
		const line = formatLine('ADD-DEVICE', { PRODUCT_NAME: 'say "hi" \\o/' })
		assert.equal(line, 'ADD-DEVICE PRODUCT_NAME="say \\"hi\\" \\\\o/"\n')
		assert.equal(parseLine(line.trimEnd()).params.PRODUCT_NAME, 'say "hi" \\o/')
	})
})

describe('parseLine', () => {
	it('parses a KEY-STATE line and keeps base64 padding intact', () => {
		const text = Buffer.from('Hello world').toString('base64')
		const msg = parseLine(
			`KEY-STATE DEVICEID="fake-01" CONTROLID="0/0" BITMAP="AAAA/w==" COLOR="#ff0000" TEXT="${text}" `
		)
		assert.equal(msg.command, 'KEY-STATE')
		assert.equal(msg.status, null)
		assert.equal(msg.params.DEVICEID, 'fake-01')
		assert.equal(msg.params.CONTROLID, '0/0')
		assert.equal(msg.params.BITMAP, 'AAAA/w==')
		assert.equal(msg.params.COLOR, '#ff0000')
		assert.equal(Buffer.from(String(msg.params.TEXT), 'base64').toString(), 'Hello world')
	})

	it('parses an OK status response', () => {
		const msg = parseLine('ADD-DEVICE OK DEVICEID="fake-01" ')
		assert.equal(msg.command, 'ADD-DEVICE')
		assert.equal(msg.status, 'OK')
		assert.equal(msg.params.DEVICEID, 'fake-01')
	})

	it('parses an ERROR response with its message', () => {
		const msg = parseLine('ADD-DEVICE ERROR DEVICEID="x" MESSAGE="Unknown control"')
		assert.equal(msg.command, 'ADD-DEVICE')
		assert.equal(msg.status, 'ERROR')
		assert.equal(msg.params.MESSAGE, 'Unknown control')
	})

	it('parses a bare ERROR with no device id', () => {
		const msg = parseLine('ERROR MESSAGE="Unknown command: FOO" ')
		assert.equal(msg.command, 'ERROR')
		assert.equal(msg.params.MESSAGE, 'Unknown command: FOO')
		assert.equal(msg.params.DEVICEID, undefined)
	})

	it('parses unknown commands into a generic message without throwing', () => {
		const msg = parseLine('FUTURE-THING A=1')
		assert.equal(msg.command, 'FUTURE-THING')
		assert.equal(msg.params.A, '1')
	})

	it('keeps the raw body for PING/PONG payloads', () => {
		const msg = parseLine('PONG abc 123')
		assert.equal(msg.command, 'PONG')
		assert.equal(msg.body, 'abc 123')
	})

	it('treats bare words as true flags', () => {
		assert.equal(parseLine('CAPS NONSQUARE').params.NONSQUARE, true)
	})
})

describe('toBool', () => {
	it('normalises 1/0/true/false', () => {
		assert.equal(toBool('1'), true)
		assert.equal(toBool('true'), true)
		assert.equal(toBool(true), true)
		assert.equal(toBool('0'), false)
		assert.equal(toBool('false'), false)
		assert.equal(toBool(undefined), false)
	})
})

describe('parseCaps', () => {
	it('parses flags and the bitmap format list', () => {
		const caps = parseCaps(parseLine('CAPS SUBSCRIPTIONS=1 NONSQUARE=1 BITMAP_FORMATS="rgb,png" ROTARY_AMOUNT=0').params)
		assert.equal(caps.subscriptions, true)
		assert.equal(caps.nonSquare, true)
		assert.equal(caps.rotaryAmount, false)
		assert.deepEqual(caps.bitmapFormats, ['rgb', 'png'])
	})

	it('defaults to rgb only when no formats are advertised', () => {
		assert.deepEqual(parseCaps({}).bitmapFormats, ['rgb'])
	})
})

describe('buildLayoutManifest', () => {
	it('declares two keys and one 128x64 encoder with text', () => {
		const manifest = JSON.parse(Buffer.from(buildLayoutManifest({ keySize: 72 }), 'base64').toString())
		assert.ok(manifest.stylePresets.default)
		assert.deepEqual(manifest.stylePresets.key.bitmap, { w: 72, h: 72 })
		assert.deepEqual(manifest.stylePresets.encoder.bitmap, { w: 128, h: 64 })
		assert.equal(manifest.stylePresets.encoder.text, true)
		assert.equal(manifest.stylePresets.encoder.textStyle, true)
		assert.deepEqual(Object.keys(manifest.controls).sort(), [...Object.values(CONTROL_IDS)].sort())
		assert.equal(manifest.controls[CONTROL_IDS.encoder].stylePreset, 'encoder')
		assert.equal(manifest.controls[CONTROL_IDS.key0].stylePreset, 'key')
		for (const id of Object.keys(manifest.controls)) assert.match(id, /^[a-zA-Z0-9\-/]+$/)
	})

	it('uses the requested key size', () => {
		const manifest = JSON.parse(Buffer.from(buildLayoutManifest({ keySize: 128 }), 'base64').toString())
		assert.deepEqual(manifest.stylePresets.key.bitmap, { w: 128, h: 128 })
	})
})

describe('summariseLine', () => {
	it('replaces a raw bitmap with its length and leaves other fields intact', () => {
		assert.equal(
			summariseLine('KEY-STATE CONTROLID="0/0" BITMAP="AAAA/w==" COLOR="#ff0000"'),
			'KEY-STATE CONTROLID="0/0" BITMAP=<8 bytes> COLOR="#ff0000"'
		)
	})

	it('marks data: URL bitmaps', () => {
		assert.equal(summariseLine('KEY-STATE BITMAP="data:image/png;base64,AA=="'), 'KEY-STATE BITMAP=<26 bytes data-url>')
	})

	it('leaves lines without a bitmap unchanged', () => {
		assert.equal(summariseLine('PONG 123'), 'PONG 123')
	})
})
