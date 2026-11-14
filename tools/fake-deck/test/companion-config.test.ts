import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'

describe('companion-config', () => {
	it('the committed page export matches make-page.ts', () => {
		const generated = execFileSync(process.execPath, ['companion-config/make-page.ts'], { encoding: 'utf8' })
		const committed = readFileSync('companion-config/fake-deck-page.companionconfig', 'utf8')
		assert.equal(generated, committed, 'regenerate with: node companion-config/make-page.ts > companion-config/fake-deck-page.companionconfig')
	})
})
