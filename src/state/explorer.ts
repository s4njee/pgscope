import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { ipc, toAppError } from '../lib/ipc'
import type { AppError, SchemaNode, TableMeta } from '../lib/types'

export interface Selection {
  schema: string
  table: string
}

interface ExplorerStore {
  tree: SchemaNode[]
  loading: boolean
  error: AppError | null

  selected: Selection | null
  meta: TableMeta | null
  metaLoading: boolean

  /** Keys of expanded tree nodes: `schema`, `schema/views`, or `db`. */
  expanded: Record<string, boolean>

  loadTree: () => Promise<void>
  select: (schema: string, table: string) => Promise<void>
  reloadMeta: () => Promise<void>
  toggleNode: (key: string) => void
  isExpanded: (key: string, fallback?: boolean) => boolean
  reset: () => void
}

export const useExplorer = create<ExplorerStore>()(
  persist(
    (set, get) => ({
      tree: [],
      loading: false,
      error: null,
      selected: null,
      meta: null,
      metaLoading: false,
      expanded: { db: true, public: true },

      loadTree: async () => {
        set({ loading: true, error: null })
        try {
          const tree = await ipc.schemaTree()
          set({ tree, loading: false })

          // `selected` is persisted but `meta` is not, so a restored selection
          // arrives with no columns. Re-selecting it is what fetches them —
          // without this the app reopens on the right table showing an empty
          // grid and "no metadata", with nothing on screen to explain why.
          const current = get().selected
          const stillExists =
            current !== null &&
            tree.some(
              (s) =>
                s.name === current.schema &&
                (s.tables.some((t) => t.name === current.table) ||
                  s.views.some((v) => v.name === current.table)),
            )

          if (current && stillExists) {
            void get().select(current.schema, current.table)
          } else {
            // No selection, or one whose table has since been dropped: land on
            // a sensible default so the app opens showing data, as the design does.
            const preferred = tree.find((s) => s.name === 'public') ?? tree[0]
            const first = preferred?.tables[0]
            if (preferred && first) {
              void get().select(preferred.name, first.name)
            }
          }
        } catch (e) {
          set({ loading: false, error: toAppError(e) })
        }
      },

      select: async (schema, table) => {
        set({ selected: { schema, table }, metaLoading: true })
        try {
          const meta = await ipc.tableMeta(schema, table)
          // Ignore a response that lost the race with a newer selection.
          const cur = get().selected
          if (cur?.schema === schema && cur?.table === table) {
            set({ meta, metaLoading: false })
          }
        } catch (e) {
          set({ metaLoading: false, error: toAppError(e) })
        }
      },

      reloadMeta: async () => {
        const sel = get().selected
        if (!sel) return
        await get().select(sel.schema, sel.table)
      },

      toggleNode: (key) => set((s) => ({ expanded: { ...s.expanded, [key]: !s.isExpanded(key) } })),

      isExpanded: (key, fallback = false) => {
        const v = get().expanded[key]
        return v === undefined ? fallback : v
      },

      reset: () => set({ tree: [], selected: null, meta: null, error: null }),
    }),
    {
      name: 'pgscope.explorer',
      // Only the durable bits: tree and metadata are refetched on connect.
      partialize: (s) => ({ expanded: s.expanded, selected: s.selected }),
    },
  ),
)
