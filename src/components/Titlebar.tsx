import { useConnection, windowTitle } from '../state/connection'
import { ipc } from '../lib/ipc'
import { formatLatency } from '../lib/format'
import type { ConnectionState } from '../lib/types'
import { useUi, type Theme } from '../state/ui'

const themes: { value: Theme; label: string; shortLabel: string }[] = [
  { value: 'dark', label: 'Dark theme', shortLabel: 'Dark' },
  { value: 'black', label: 'Black theme', shortLabel: 'Black' },
  { value: 'light', label: 'Light theme', shortLabel: 'Light' },
]

/**
 * Text for the connection pill. A null latency means no ping has come back yet,
 * which reads as plain "connected" rather than a placeholder number.
 *
 * @param state - `ConnectionState` — one of `'disconnected' | 'connecting' | 'connected' |
 *   'lost'`; `'lost'` renders as "reconnecting…".
 * @param latencyMs - `number | null` — last round-trip ping in milliseconds; `null` means
 *   no ping has come back yet, so no latency is shown.
 * @returns `string` — the pill's user-facing label, e.g. `connected · 12 ms`.
 */
function pillLabel(state: ConnectionState, latencyMs: number | null): string {
  switch (state) {
    case 'connected':
      return latencyMs === null ? 'connected' : `connected · ${formatLatency(latencyMs)}`
    case 'connecting':
      return 'connecting…'
    case 'lost':
      return 'reconnecting…'
    default:
      return 'disconnected'
  }
}

/** The pill's status dot. A lost connection shares the connecting state's
 *  pulse, because that is what it is doing — reconnecting.
 *
 * @param state - `ConnectionState` — one of `'disconnected' | 'connecting' | 'connected' |
 *   'lost'`.
 * @returns `string` — the dot's `className`; the base `conn-pill__dot` plus a
 *   `--connecting` or `--down` modifier for the non-connected states.
 */
function pillDotClass(state: ConnectionState): string {
  if (state === 'connected') return 'conn-pill__dot'
  if (state === 'connecting' || state === 'lost') return 'conn-pill__dot conn-pill__dot--connecting'
  return 'conn-pill__dot conn-pill__dot--down'
}

interface Props {
  platform: string
}

/**
 * 42px titlebar. On macOS the native traffic lights are overlaid by the window
 * config, so we only reserve space; elsewhere we draw them and wire the window
 * controls ourselves.
 *
 * @param props - `Props` — `{ platform: string }`
 *   - `platform` — the Tauri OS identifier; the exact value `'macos'` switches on the
 *     reserved-space layout, anything else gets drawn traffic lights.
 * @returns `JSX.Element` — the titlebar row: traffic lights (non-macOS only), the window
 *   title, and the connection pill that opens the connection modal.
 */
export function Titlebar({ platform }: Props) {
  const { state, info, latencyMs, openModal } = useConnection()
  const theme = useUi((s) => s.theme)
  const setTheme = useUi((s) => s.setTheme)
  const isMac = platform === 'macos'

  return (
    <div
      className={`titlebar${isMac ? ' titlebar--macos' : ''}`}
      data-tauri-drag-region
    >
      {!isMac && (
        <div className="traffic-lights">
          <button
            className="traffic-light traffic-light--close"
            aria-label="Close window"
            onClick={() => void ipc.windowClose()}
          />
          <button
            className="traffic-light traffic-light--min"
            aria-label="Minimize window"
            onClick={() => void ipc.windowMinimize()}
          />
          <button
            className="traffic-light traffic-light--max"
            aria-label="Maximize window"
            onClick={() => void ipc.windowToggleMaximize()}
          />
        </div>
      )}

      <div className="titlebar__title" data-tauri-drag-region>
        {windowTitle(info)}
      </div>

      <div className="theme-switcher" role="group" aria-label="Color theme">
        {themes.map((option) => (
          <button
            key={option.value}
            className={`theme-switcher__option${
              theme === option.value ? ' theme-switcher__option--active' : ''
            }`}
            aria-label={option.label}
            aria-pressed={theme === option.value}
            title={option.label}
            onClick={() => setTheme(option.value)}
          >
            {option.shortLabel}
          </button>
        ))}
      </div>

      <button
        className="conn-pill"
        onClick={openModal}
        title="Connection settings"
      >
        <span className={pillDotClass(state)} />
        <span className="conn-pill__label">{pillLabel(state, latencyMs)}</span>
      </button>
    </div>
  )
}
