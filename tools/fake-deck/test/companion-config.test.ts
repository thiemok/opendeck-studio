import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { benchControlIds } from '../src/protocol.ts'

describe('companion-config', () => {
	it('the committed page export matches make-page.ts', () => {
		const generated = execFileSync(process.execPath, ['companion-config/make-page.ts'], { encoding: 'utf8' })
		const committed = readFileSync('companion-config/fake-deck-page.companionconfig', 'utf8')
		assert.equal(generated, committed, 'regenerate with: node companion-config/make-page.ts > companion-config/fake-deck-page.companionconfig')
	})

	it('the committed hardware page matches make-page.ts --hardware', () => {
		const generated = execFileSync(process.execPath, ['companion-config/make-page.ts', '--hardware'], { encoding: 'utf8' })
		assert.equal(generated, readFileSync('companion-config/hardware-deck-page.companionconfig', 'utf8'))
	})

	describe('bench pages (THI-76)', () => {
		const benchPage = (n: number) => {
			const text = execFileSync(process.execPath, ['companion-config/make-page.ts', '--bench', String(n)], {
				encoding: 'utf8',
			})
			assert.equal(text, readFileSync(`companion-config/bench-page-${n}.companionconfig`, 'utf8'), `regenerate page ${n}`)
			return JSON.parse(text).page
		}
		type Button = { type: string; style?: { layers: { id: string; color?: { value: number }; text?: { value: string } }[] } }
		const buttons = (page: { controls: Record<string, Record<string, Button>> }) =>
			Object.entries(page.controls).flatMap(([row, cols]) =>
				Object.entries(cols).map(([col, b]) => ({ id: `${row}/${col}`, b }))
			)
		const layer = (b: Button, id: string) => b.style?.layers.find((l) => l.id === id)

		it('fill all 32 positions, with page up at 0/0 and page down at 0/1', () => {
			for (const n of [1, 2]) {
				const all = buttons(benchPage(n))
				assert.equal(all.length, 32)
				assert.deepEqual(all.map((x) => x.id).sort(), benchControlIds(32).sort())
				assert.deepEqual(all.find((x) => x.id === '0/0')?.b, { type: 'pageup' })
				assert.deepEqual(all.find((x) => x.id === '0/1')?.b, { type: 'pagedown' })
			}
		})

		it('give the other 30 buttons distinct colours, and change every colour and text between pages', () => {
			const [one, two] = [benchPage(1), benchPage(2)].map((p) =>
				new Map(
					buttons(p)
						.filter((x) => x.b.type === 'button-layered')
						.map((x) => [x.id, { color: layer(x.b, 'box0')?.color?.value, text: layer(x.b, 'text0')?.text?.value }])
				)
			)
			assert.equal(one.size, 30)
			assert.equal(new Set([...one.values()].map((v) => v.color)).size, 30)
			for (const [id, a] of one) {
				const b = two.get(id)
				assert.ok(b, id)
				assert.notEqual(a.color, b.color, id)
				assert.notEqual(a.text, b.text, id)
			}
		})

		it('reject other page numbers', () => {
			assert.throws(() =>
				execFileSync(process.execPath, ['companion-config/make-page.ts', '--bench', '3'], { stdio: 'pipe' })
			)
		})
	})
})
