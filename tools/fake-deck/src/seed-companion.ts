// Loads a test layout into a running Companion, so fresh Companion data needs no clicks in
// the web UI: it marks the first-run wizard done, creates the custom variables the pages use and
// imports the page files from `companion-config/`.
//
// Companion has no HTTP endpoint for imports, so this speaks the tRPC-over-WebSocket API that
// Companion's own web UI uses (`/trpc`, JSON with no transformer). Those procedure names are not
// a public API; they match the Companion version pinned in compose.yaml (5.1.3). Bump both
// together and rerun `npm run seed` against the new version.

import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { setTimeout as sleep } from 'node:timers/promises'
import { parseArgs } from 'node:util'

/** Each layout is a list of (page file, target page). Target -1 inserts a new page at the end. */
const LAYOUTS: Record<string, { file: string; page: number }[]> = {
	hardware: [{ file: 'hardware-deck-page.companionconfig', page: 1 }],
	fake: [{ file: 'fake-deck-page.companionconfig', page: 1 }],
	bench: [
		{ file: 'bench-page-1.companionconfig', page: 1 },
		{ file: 'bench-page-2.companionconfig', page: 2 },
	],
}
const VARIABLES = ['count', 'presses']
/** `setup_wizard` stores the wizard version last completed; Companion 5.1.3's current one is 50. */
const WIZARD_VERSION = 50
const CHUNK_BYTES = 64 * 1024

const HELP = `Usage: npm run seed -- [options]

Loads a test layout into Companion: skips the first-run wizard, creates the custom
variables ${VARIABLES.join(', ')} and replaces the layout's pages with the files in companion-config/.

Options:
  --layout <name>    ${Object.keys(LAYOUTS).join(' | ')} (default: hardware)
  --url <url>        Companion web UI (default: http://127.0.0.1:18000)
  --wait <seconds>   How long to wait for Companion to come up (default: 90)
  --if-fresh         Only seed a Companion that has never been set up (first-run wizard
                     not done), so pages edited in the web UI are kept
  -h, --help
`

type Pending = { resolve: (data: unknown) => void; reject: (err: Error) => void }

/** A minimal tRPC v11 WebSocket client: queries and mutations only, one call at a time is enough. */
class Trpc {
	#ws: WebSocket
	#nextId = 1
	#pending = new Map<number, Pending>()

	private constructor(ws: WebSocket) {
		this.#ws = ws
		ws.addEventListener('message', (event) => {
			const msg = JSON.parse(String(event.data))
			const call = this.#pending.get(msg.id)
			if (!call) return
			this.#pending.delete(msg.id)
			if (msg.error) call.reject(new Error(`${msg.error.message ?? JSON.stringify(msg.error)}`))
			else call.resolve(msg.result?.data)
		})
		ws.addEventListener('close', () => {
			for (const call of this.#pending.values()) call.reject(new Error('Companion closed the connection'))
			this.#pending.clear()
		})
	}

	static connect(url: string): Promise<Trpc> {
		return new Promise((resolve, reject) => {
			const ws = new WebSocket(url)
			ws.addEventListener('open', () => resolve(new Trpc(ws)), { once: true })
			ws.addEventListener('error', () => reject(new Error(`cannot connect to ${url}`)), { once: true })
		})
	}

	call(method: 'query' | 'mutation', path: string, input?: unknown): Promise<unknown> {
		const id = this.#nextId++
		return new Promise((resolve, reject) => {
			this.#pending.set(id, { resolve, reject })
			this.#ws.send(JSON.stringify({ id, method, params: { path, input } }))
		})
	}

	close() {
		this.#ws.close()
	}
}

async function connectWithRetry(url: string, waitSeconds: number): Promise<Trpc> {
	const deadline = Date.now() + waitSeconds * 1000
	for (;;) {
		try {
			return await Trpc.connect(url)
		} catch (err) {
			if (Date.now() > deadline) throw err
			await sleep(1000)
		}
	}
}

/** Uploads a page export the way the web UI's import does, then puts its page 1 at `targetPage`. */
async function importPage(trpc: Trpc, file: string, targetPage: number) {
	const data = readFileSync(new URL(`../companion-config/${file}`, import.meta.url))
	const sessionId = await trpc.call('mutation', 'importExport.prepareImport.start', { name: file, size: data.length })
	for (let offset = 0; offset < data.length; offset += CHUNK_BYTES) {
		const chunk = data.subarray(offset, offset + CHUNK_BYTES).toString('base64')
		await trpc.call('mutation', 'importExport.prepareImport.uploadChunk', { sessionId, offset, data: chunk })
	}
	const expectedChecksum = createHash('sha1').update(data).digest('hex')
	const [error] = (await trpc.call('mutation', 'importExport.prepareImport.complete', {
		sessionId,
		expectedChecksum,
		userData: null,
	})) as [string | null, unknown]
	if (error) throw new Error(`Companion rejected ${file}: ${error}`)
	const importTo = (page: number) =>
		trpc.call('mutation', 'importExport.importSinglePage', { targetPage: page, sourcePage: 1, connectionIdRemapping: {} })
	try {
		await importTo(targetPage)
	} catch (err) {
		// A fresh Companion has only page 1, so page 2 of the bench layout does not exist yet.
		// -1 appends a page, which lands on `targetPage` only when it is the next free page (true
		// for the layouts above, which fill pages in order). Reruns then replace it in place.
		if (!/invalid target page/i.test((err as Error).message)) throw err
		await importTo(-1)
	}
}

async function main() {
	const { values } = parseArgs({
		options: {
			layout: { type: 'string', default: 'hardware' },
			url: { type: 'string', default: 'http://127.0.0.1:18000' },
			wait: { type: 'string', default: '90' },
			'if-fresh': { type: 'boolean' },
			help: { type: 'boolean', short: 'h' },
		},
	})
	if (values.help) {
		process.stdout.write(HELP)
		return
	}
	const layout = LAYOUTS[values.layout]
	if (!layout) throw new Error(`unknown layout "${values.layout}"; pick one of ${Object.keys(LAYOUTS).join(', ')}`)

	const wsUrl = new URL('/trpc', values.url.replace(/^http/, 'ws')).toString()
	const trpc = await connectWithRetry(wsUrl, Number(values.wait))
	try {
		const config = (await trpc.call('query', 'userConfig.getConfig')) as { setup_wizard: number }
		if (values['if-fresh'] && config.setup_wizard !== 0) {
			console.log('Companion is already set up; not seeding (run `npm run seed` to replace its pages)')
			return
		}
		if (config.setup_wizard < WIZARD_VERSION) {
			await trpc.call('mutation', 'userConfig.setConfigKey', { key: 'setup_wizard', value: WIZARD_VERSION })
			console.log('first-run wizard marked done (Companion defaults kept)')
		}

		for (const name of VARIABLES) {
			// Returns an error message instead of throwing, e.g. when the variable already exists.
			const problem = await trpc.call('mutation', 'customVariables.create', { name, defaultVal: '0' })
			console.log(problem ? `custom variable ${name}: ${problem}` : `custom variable ${name}: created`)
		}

		for (const { file, page } of layout) {
			await importPage(trpc, file, page)
			console.log(`page ${page}: imported ${file}`)
		}
	} finally {
		trpc.close()
	}
}

main().catch((err: Error) => {
	console.error(`seed failed: ${err.message}`)
	process.exit(1)
})
