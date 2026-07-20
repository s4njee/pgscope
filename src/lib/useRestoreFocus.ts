import { useEffect, useRef } from 'react'

/**
 * Hands focus back to whatever held it before a modal opened. Without this the
 * caller (the CodeMirror editor, the terminal's hidden input, the connection
 * pill) is left unfocused and the next thing typed goes nowhere.
 */
export function useRestoreFocus(open: boolean) {
  const restoreTo = useRef<HTMLElement | null>(null)

  useEffect(() => {
    if (!open) return
    restoreTo.current = document.activeElement as HTMLElement | null

    return () => {
      const target = restoreTo.current
      restoreTo.current = null
      // The element may have been unmounted while the modal was open.
      if (target?.isConnected) {
        requestAnimationFrame(() => target.focus())
      }
    }
  }, [open])
}
