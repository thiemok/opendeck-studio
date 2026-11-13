// Generates fake-deck-page.companionconfig: a Companion 5.1 page export for the fake deck's
// three controls (row 0: key A, key B, encoder). Import it on page 1 via Import / Export.
//   node companion-config/make-page.ts > companion-config/fake-deck-page.companionconfig
//
// All three controls drive the custom variable `count` (create it under Variables first):
//   key A (0/0)  press: count + 1         text "A <count>" on red
//   key B (0/1)  press: count - 1         text "B <count>" on blue
//   encoder (0/2) rotate right/left: count +/- 1, push: count = 0, text "Vol <count>"

const v = <T>(value: T, isExpression = false) => ({ value, isExpression })

const setCount = (expression: string) => ({
	id: `set-${Math.random().toString(36).slice(2, 10)}`,
	definitionId: 'custom_variable_set_value',
	connectionId: 'internal',
	type: 'action',
	children: {},
	options: { name: v('count'), create: v(false), value: v(expression, true) },
})

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

const page = {
	version: 12,
	type: 'page',
	companionBuild: '5.1.3+10036-stable-93ea3fa61c',
	page: {
		id: 'fakeDeckPage00000000',
		name: 'FAKE DECK',
		controls: {
			'0': {
				'0': button('A\n$(custom:count)', 0xcc0000, { down: [setCount('$(custom:count) + 1')] }),
				'1': button('B\n$(custom:count)', 0x0044cc, { down: [setCount('$(custom:count) - 1')] }),
				'2': button(
					'Vol $(custom:count)',
					0x000000,
					{
						down: [setCount('0')],
						rotate_left: [setCount('$(custom:count) - 1')],
						rotate_right: [setCount('$(custom:count) + 1')],
					},
					true
				),
			},
		},
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
