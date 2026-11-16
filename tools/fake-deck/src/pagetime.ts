/**
 * Page-change timing on a bench surface; mirrors `crates/deck-core/src/pagetime.rs` (THI-76).
 *
 * A burst starts at the first key image and takes the oldest unanswered press as its trigger.
 * It ends complete once every key has been drawn, or partial after QUIET_MS without an image.
 */

export const QUIET_MS = 1000
export const PRESS_MAX_AGE_MS = 5000
export const SUMMARY_EVERY = 10
const WINDOW = 64

export interface PageChange {
	keys: number
	drawn: number
	/** From the triggering press to the last key drawn; null when no press triggered it. */
	pressToLastMs: number | null
	firstToLastMs: number
	images: number
	wireBytes: number
	receiveMsSum: number
	receiveMsMax: number
	decodeMsSum: number
	decodeMsMax: number
	complete: boolean
}

export interface DrawnImage {
	key: number
	lineStartedMs: number
	doneMs: number
	wireBytes: number
	receiveMs: number
	decodeMs: number
}

interface Burst {
	startedMs: number
	pressedMs: number | null
	seen: Set<number>
	lastMs: number
	change: PageChange
}

export class PageTimer {
	readonly keys: number
	#pressedMs: number | null = null
	#burst: Burst | null = null
	#samples: number[] = []
	#sinceSummary = 0

	constructor(keys: number) {
		this.keys = keys
	}

	pressed(nowMs: number): void {
		if (this.#pressedMs !== null && nowMs - this.#pressedMs <= PRESS_MAX_AGE_MS) return
		this.#pressedMs = nowMs
	}

	drawn(image: DrawnImage): PageChange | null {
		if (image.key < 0 || image.key >= this.keys) return null
		if (!this.#burst) {
			const pressed = this.#pressedMs
			this.#pressedMs = null
			this.#burst = {
				startedMs: image.lineStartedMs,
				pressedMs: pressed !== null && image.lineStartedMs - pressed <= PRESS_MAX_AGE_MS ? pressed : null,
				seen: new Set(),
				lastMs: image.doneMs,
				change: {
					keys: this.keys,
					drawn: 0,
					pressToLastMs: null,
					firstToLastMs: 0,
					images: 0,
					wireBytes: 0,
					receiveMsSum: 0,
					receiveMsMax: 0,
					decodeMsSum: 0,
					decodeMsMax: 0,
					complete: false,
				},
			}
		}
		const burst = this.#burst
		burst.seen.add(image.key)
		burst.lastMs = image.doneMs
		const c = burst.change
		c.drawn = burst.seen.size
		c.images++
		c.wireBytes += image.wireBytes
		c.receiveMsSum += image.receiveMs
		c.receiveMsMax = Math.max(c.receiveMsMax, image.receiveMs)
		c.decodeMsSum += image.decodeMs
		c.decodeMsMax = Math.max(c.decodeMsMax, image.decodeMs)
		return c.drawn === this.keys ? this.#finish() : null
	}

	tick(nowMs: number): PageChange | null {
		if (this.#burst && nowMs - this.#burst.lastMs >= QUIET_MS) return this.#finish()
		return null
	}

	#finish(): PageChange | null {
		const burst = this.#burst
		if (!burst) return null
		this.#burst = null
		const change = {
			...burst.change,
			firstToLastMs: burst.lastMs - burst.startedMs,
			pressToLastMs: burst.pressedMs === null ? null : burst.lastMs - burst.pressedMs,
			complete: burst.change.drawn === this.keys,
		}
		if (change.complete && change.pressToLastMs !== null) {
			this.#samples.push(change.pressToLastMs)
			if (this.#samples.length > WINDOW) this.#samples.shift()
			this.#sinceSummary++
		}
		return change
	}

	summaryDue(): boolean {
		if (this.#sinceSummary < SUMMARY_EVERY) return false
		this.#sinceSummary = 0
		return true
	}

	/** Median of the window; the mean of the middle two, rounded down, when the count is even. */
	median(): number | null {
		const n = this.#samples.length
		if (n === 0) return null
		const sorted = [...this.#samples].sort((a, b) => a - b)
		const mid = Math.floor(n / 2)
		return n % 2 ? sorted[mid] : Math.floor((sorted[mid - 1] + sorted[mid]) / 2)
	}

	max(): number | null {
		return this.#samples.length ? Math.max(...this.#samples) : null
	}
}

export function formatPageChange(c: PageChange, format: string): string {
	const press = c.pressToLastMs === null ? 'none' : `${c.pressToLastMs} ms`
	return (
		`page change ${c.drawn}/${c.keys} keys ${format}: press to last ${press}, first to last ${c.firstToLastMs} ms, ` +
		`${c.images} images ${c.wireBytes} B, receive sum ${c.receiveMsSum} max ${c.receiveMsMax} ms, ` +
		`decode sum ${c.decodeMsSum} max ${c.decodeMsMax} ms${c.complete ? '' : ' (partial)'}`
	)
}
