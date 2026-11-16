import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { describe, it } from 'node:test'
import { type DrawnImage, formatPageChange, PageTimer, PRESS_MAX_AGE_MS, QUIET_MS } from '../src/pagetime.ts'

// The same scenarios as crates/deck-core/src/pagetime.rs, so the two timers agree.
const image = (key: number, start: number, done: number): DrawnImage => ({
	key,
	lineStartedMs: start,
	doneMs: done,
	wireBytes: 1000 + key,
	receiveMs: 20 + key,
	decodeMs: 5,
})

/** Draws keys 0..n, 25 ms apart from start; returns what the last one completed. */
const page = (timer: PageTimer, n: number, start: number) => {
	let last = null
	for (let k = 0; k < n; k++) last = timer.drawn(image(k, start + k * 25, start + k * 25 + 20))
	return last
}

const range = (n: number) => Array.from({ length: n }, (_, i) => i).reduce((a, b) => a + b, 0)

describe('PageTimer', () => {
	it('a press then every key gives one complete summary', () => {
		const timer = new PageTimer(32)
		timer.pressed(0)
		const change = page(timer, 32, 100)
		assert.ok(change)
		assert.equal(change.pressToLastMs, 895)
		assert.equal(change.firstToLastMs, 795)
		assert.equal(change.complete, true)
		assert.equal(change.wireBytes, 32 * 1000 + range(32))
		assert.equal(change.receiveMsSum, 32 * 20 + range(32))
		assert.equal(change.receiveMsMax, 51)
		assert.deepEqual([change.decodeMsSum, change.decodeMsMax], [160, 5])
		assert.equal(timer.tick(10_000), null)
	})

	it('a key drawn twice counts once toward completion', () => {
		const timer = new PageTimer(16)
		assert.equal(timer.drawn(image(3, 0, 10)), null)
		assert.equal(timer.drawn(image(3, 10, 20)), null)
		const change = page(timer, 16, 30)
		assert.deepEqual([change?.drawn, change?.images], [16, 18])
	})

	it('images without a press start at the first line', () => {
		const timer = new PageTimer(16)
		const change = page(timer, 16, 500)
		assert.equal(change?.pressToLastMs, null)
		assert.equal(change?.firstToLastMs, 15 * 25 + 20)
		assert.equal(timer.median(), null)
	})

	it('a quiet second ends a partial page change', () => {
		const timer = new PageTimer(32)
		timer.pressed(0)
		assert.equal(page(timer, 20, 100), null)
		const lastDone = 100 + 19 * 25 + 20
		assert.equal(timer.tick(lastDone + QUIET_MS - 1), null)
		const change = timer.tick(lastDone + QUIET_MS)
		assert.deepEqual([change?.drawn, change?.keys, change?.complete], [20, 32, false])
		assert.match(formatPageChange(change!, 'png'), /^page change 20\/32 keys png: .*\(partial\)$/)
	})

	it('a stale press is not the trigger', () => {
		const timer = new PageTimer(16)
		timer.pressed(0)
		assert.equal(page(timer, 16, PRESS_MAX_AGE_MS + 1)?.pressToLastMs, null)
	})

	it('a summary is due every ten timed page changes', () => {
		const timer = new PageTimer(16)
		const due: number[] = []
		for (let i = 0; i < 20; i++) {
			timer.pressed(i * 10_000)
			page(timer, 16, i * 10_000 + 100 + i)
			if (timer.summaryDue()) due.push(i + 1)
		}
		assert.deepEqual(due, [10, 20])
		assert.equal(timer.median(), 504)
		assert.equal(timer.max(), 514)
	})
})

describe('CLI', () => {
	it('--keys accepts only 16 or 32', () => {
		for (const keys of ['12', '0', 'many']) {
			let status = 0
			let stderr = ''
			try {
				execFileSync(process.execPath, ['src/main.ts', '--keys', keys], { stdio: 'pipe', encoding: 'utf8' })
			} catch (err) {
				const e = err as { status: number; stderr: string }
				status = e.status
				stderr = e.stderr
			}
			assert.equal(status, 2, keys)
			assert.match(stderr, /--keys must be 16 or 32/)
		}
	})
})
