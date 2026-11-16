import { CONTROL_IDS } from '../src/protocol.ts'

const v = <T>(value: T, isExpression = false) => ({ value, isExpression })

let actionCount = 0
const setVariable = (name: string, expression: string) => ({
	id: `set-${name}-${++actionCount}`,
	definitionId: 'custom_variable_set_value',
	connectionId: 'internal',
	type: 'action',
	children: {},
	options: { name: v(name), create: v(false), value: v(expression, true) },
})
const setCount = (expression: string) => setVariable('count', expression)

const button = (text: string, bg: number, actionSets: Record<string, unknown[]>, rotaryActions = false) => ({
	type: 'button-layered',
	style: {
		layers: [
			{ id: 'canvas', name: 'Canvas', usage: 'auto', type: 'canvas', decoration: v('default'), showStatusIcons: v('default') },
			{
				id: 'box0',
				name: 'Background',
				usage: 'auto',
				type: 'box',
				pinnedProperties: ['color'],
				enabled: v(true),
				opacity: v(100),
				x: v(0),
				y: v(0),
				width: v(100),
				height: v(100),
				rotation: v(0),
				color: v(bg),
				cornerRadius: v(0),
				borderWidth: v(0),
				borderColor: v(0),
				borderPosition: v('inside'),
			},
			{
				id: 'text0',
				name: 'Text',
				usage: 'auto',
				type: 'text',
				pinnedProperties: ['text', 'fontsize', 'fontsizeAllowShrink', 'color', 'halign', 'valign'],
				enabled: v(true),
				opacity: v(100),
				x: v(0),
				y: v(0),
				width: v(100),
				height: v(100),
				rotation: v(0),
				text: v(text),
				color: v(0xffffff),
				halign: v('center'),
				valign: v('center'),
				fontsize: v(100),
				fontsizeAllowShrink: v(true),
				font: v('companion-sans'),
				weight: v('normal'),
				styles: v([]),
				outlineColor: v(4278190080),
			},
		],
	},
	options: { stepProgression: 'auto', stepExpression: '', rotaryActions, canModifyStyleInApis: false, notes: '' },
	feedbacks: [],
	steps: { '0': { action_sets: { up: [], ...actionSets }, options: { runWhileHeld: [] } } },
	localVariables: [],
})

// `--hardware` lays the page out for the firmware deck instead, which leaves column 0 to
// Companion's page buttons (KTD12, KTD20 in the PoC plan).
// The keys there count their own presses in `presses`, so a turn redraws only the OLED
// and a press redraws only the keys: each input changes as few images as real use does.
const HARDWARE_IDS = { key0: '0/1', key1: '0/2', encoder: '0/3' }
const hardware = process.argv.includes('--hardware')
const ids = hardware ? HARDWARE_IDS : CONTROL_IDS
const keyVar = hardware ? 'presses' : 'count'

// Companion page exports key controls by row, then column; place each at the deck's own control ID.
const controls: Record<string, Record<string, unknown>> = {}
const place = (controlId: string, control: unknown) => {
	const [row, column] = controlId.split('/')
	controls[row] ??= {}
	controls[row][column] = control
}
place(
	ids.key0,
	button(`A\n$(custom:${keyVar})`, 0xcc0000, { down: [setVariable(keyVar, `$(custom:${keyVar}) + 1`)] })
)
place(
	ids.key1,
	button(`B\n$(custom:${keyVar})`, 0x0044cc, { down: [setVariable(keyVar, `$(custom:${keyVar}) - 1`)] })
)
place(
	ids.encoder,
	button(
		'Vol $(custom:count)',
		0x000000,
		{
			down: [setCount('0')],
			rotate_left: [setCount('$(custom:count) - 1')],
			rotate_right: [setCount('$(custom:count) + 1')],
		},
		true
	)
)

const page = {
	version: 12,
	type: 'page',
	companionBuild: '5.1.3+10036-stable-93ea3fa61c',
	page: {
		id: 'fakeDeckPage00000000',
		name: 'FAKE DECK',
		controls,
		gridSize: { minColumn: 0, maxColumn: 7, minRow: 0, maxRow: 3 },
		pageVariables: [],
	},
	instances: {},
	connectionCollections: [],
	oldPageNumber: 1,
	imageLibrary: [],
	imageLibraryCollections: [],
}

process.stdout.write(`${JSON.stringify(page, null, '\t')}\n`)
