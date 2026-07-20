import { useEffect, useState } from 'react'

import { ipc } from '../lib/ipc'
import type { Profile, SslMode } from '../lib/types'
import { useRestoreFocus } from '../lib/useRestoreFocus'
import { useConnection } from '../state/connection'

/**
 * A new profile pre-filled with a stock local server, so the form starts usable.
 *
 * Takes no arguments.
 *
 * @returns `Profile` — a fresh profile carrying a newly minted UUID `id`, pointed at
 *   `postgres@localhost:5432/postgres` with `sslmode: 'prefer'`. Not yet persisted.
 */
function blankProfile(): Profile {
  return {
    id: crypto.randomUUID(),
    name: 'local',
    host: 'localhost',
    port: 5432,
    database: 'postgres',
    user: 'postgres',
    sslmode: 'prefer',
  }
}

/**
 * Connect / profile manager. Not in the design mock — styled from the same
 * tokens so it reads as part of the app.
 *
 * Takes no arguments — everything it renders comes from the connection store.
 *
 * @returns `JSX.Element | null` — the dialog, or `null` while the store's `modalOpen`
 *   is false.
 */
export function ConnectModal() {
  const { modalOpen, closeModal, connect, state, error, info } = useConnection()
  const [profiles, setProfiles] = useState<Profile[]>([])
  const [draft, setDraft] = useState<Profile>(blankProfile)
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)

  useRestoreFocus(modalOpen)

  useEffect(() => {
    if (!modalOpen) return
    ipc
      .listProfiles()
      .then((list) => {
        setProfiles(list)
        if (list.length > 0) setDraft(list[0])
      })
      .catch(() => setProfiles([]))
  }, [modalOpen])

  // Escape closes, as in every other dialog here. Capturing, so it wins over
  // the window-level Escape handlers in the grid and the terminal.
  useEffect(() => {
    if (!modalOpen) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        closeModal()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [modalOpen, closeModal])

  if (!modalOpen) return null

  const set = <K extends keyof Profile>(key: K, value: Profile[K]) =>
    setDraft((d) => ({ ...d, [key]: value }))

  const saveAndConnect = async () => {
    setBusy(true)
    try {
      await ipc.saveProfile(draft, password || undefined)
      await connect(draft.id, password || undefined)
    } finally {
      setBusy(false)
      setPassword('')
    }
  }

  const remove = async (id: string) => {
    await ipc.deleteProfile(id)
    const list = await ipc.listProfiles()
    setProfiles(list)
    if (draft.id === id) setDraft(list[0] ?? blankProfile())
  }

  return (
    <div className="modal-scrim" onClick={(e) => e.target === e.currentTarget && closeModal()}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="connect-title"
      >
        <h2 id="connect-title" className="modal__title">
          Connect
        </h2>
        <p className="modal__subtitle">
          {info ? `Connected to ${info.database}@${info.host}:${info.port}` : 'No active connection'}
        </p>

        {profiles.length > 0 && (
          <div className="profile-list">
            {profiles.map((p) => (
              <button
                key={p.id}
                className={`profile-row${p.id === draft.id ? ' profile-row--active' : ''}`}
                onClick={() => setDraft(p)}
              >
                <span>{p.name}</span>
                <span className="profile-row__meta">
                  {p.user}@{p.host}:{p.port}/{p.database}
                </span>
                <span
                  className="profile-row__delete"
                  onClick={(e) => {
                    e.stopPropagation()
                    void remove(p.id)
                  }}
                  title="Delete profile"
                >
                  ✕
                </span>
              </button>
            ))}
          </div>
        )}

        <div className="field">
          <label className="field__label" htmlFor="cm-name">NAME</label>
          <input
            id="cm-name"
            className="field__input"
            value={draft.name}
            onChange={(e) => set('name', e.target.value)}
          />
        </div>

        <div className="field-row">
          <div className="field">
            <label className="field__label" htmlFor="cm-host">HOST</label>
            <input
              id="cm-host"
              className="field__input"
              value={draft.host}
              onChange={(e) => set('host', e.target.value)}
            />
          </div>
          <div className="field" style={{ maxWidth: 90 }}>
            <label className="field__label" htmlFor="cm-port">PORT</label>
            <input
              id="cm-port"
              className="field__input"
              type="number"
              value={draft.port}
              onChange={(e) => set('port', Number(e.target.value) || 5432)}
            />
          </div>
        </div>

        <div className="field-row">
          <div className="field">
            <label className="field__label" htmlFor="cm-db">DATABASE</label>
            <input
              id="cm-db"
              className="field__input"
              value={draft.database}
              onChange={(e) => set('database', e.target.value)}
            />
          </div>
          <div className="field">
            <label className="field__label" htmlFor="cm-user">USER</label>
            <input
              id="cm-user"
              className="field__input"
              value={draft.user}
              onChange={(e) => set('user', e.target.value)}
            />
          </div>
        </div>

        <div className="field-row">
          <div className="field">
            <label className="field__label" htmlFor="cm-pw">PASSWORD</label>
            <input
              id="cm-pw"
              className="field__input"
              type="password"
              placeholder="stored in the OS keychain"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && void saveAndConnect()}
            />
          </div>
          <div className="field" style={{ maxWidth: 110 }}>
            <label className="field__label" htmlFor="cm-ssl">SSL</label>
            <select
              id="cm-ssl"
              className="field__input"
              value={draft.sslmode}
              onChange={(e) => set('sslmode', e.target.value as SslMode)}
            >
              <option value="prefer">prefer</option>
              <option value="require">require</option>
              <option value="disable">disable</option>
            </select>
          </div>
        </div>

        {error && (
          <div className="modal__error">
            {error.message}
            {error.detail ? ` — ${error.detail}` : ''}
          </div>
        )}

        <div className="modal__actions">
          {/* Always offered, including with no connection: this dialog opens by
              itself at launch when connecting fails, and gating the exit on
              `info` left that case with no way out at all. Closing is safe —
              the titlebar's connection pill reopens it. */}
          <button className="btn-ghost" onClick={closeModal}>
            Cancel
          </button>
          <button
            className="btn-accent"
            onClick={() => void saveAndConnect()}
            disabled={busy || state === 'connecting'}
          >
            {busy || state === 'connecting' ? 'Connecting…' : 'Save & connect'}
          </button>
        </div>
      </div>
    </div>
  )
}
