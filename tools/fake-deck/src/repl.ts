import { CONTROL_IDS } from './protocol.ts'
import type { FakeDeckSession } from './session.ts'

/** Runs one REPL command. Resolves to false for `quit`. */
export async function executeCommand(session: FakeDeckSession, input: string): Promise<boolean> {
	const [cmd, arg] = input.trim().split(/\s+/)
	switch (cmd) {
		case 'press': {
			const key = arg === '0' ? CONTROL_IDS.key0 : arg === '1' ? CONTROL_IDS.key1 : null
			if (!key) {
				console.log('usage: press <0|1>')
				break
			}
			await session.press(key)
			break
		}
		case 'push':
			await session.press(CONTROL_IDS.encoder)
			break
		case 'rotate': {
			const steps = Number(arg)
			if (!Number.isInteger(steps) || steps === 0) {
				console.log('usage: rotate <+n|-n>')
				break
			}
			// One KEY-ROTATE per detent: Companion runs the rotate action once per message, whatever DIRECTION's magnitude.
			for (let i = 0; i < Math.abs(steps); i++) session.rotate(CONTROL_IDS.encoder, Math.sign(steps))
			break
		}
		case 'quit':
			return false
		case undefined:
		case '':
			break
		default:
			console.log(`unknown command: ${cmd} (press, push, rotate, quit)`)
	}
	return true
}
