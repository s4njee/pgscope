/**
 * Regenerate the README screenshots from the demo UI.
 *
 * Runs the real frontend against `src/lib/demo.ts`'s in-memory fixtures — the
 * same components and stores the Tauri app uses, just with the IPC layer served
 * from memory instead of Rust. That is deliberate: a screenshot of a mock-up
 * goes stale silently, whereas this breaks loudly when a component changes.
 *
 * Usage:
 *   pnpm dev            # in another shell, serving on :1425
 *   node scripts/screenshots.mjs
 *
 * Requires Chrome; no browser is downloaded (`puppeteer-core` drives the one
 * already installed).
 */

import { mkdir } from 'node:fs/promises'
import { join } from 'node:path'
import puppeteer from 'puppeteer-core'

const URL = process.env.PGSCOPE_DEMO_URL ?? 'http://localhost:1425'
const OUT = join(process.cwd(), 'docs', 'screenshots')

/** Design width plus enough height for the grid and the docked psql pane. */
const VIEWPORT = { width: 1440, height: 900, deviceScaleFactor: 2 }

const CHROME_CANDIDATES = [
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium',
  '/usr/bin/google-chrome',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
]

/**
 * Locate an installed Chrome.
 *
 * @returns `Promise<string>` — absolute path to the executable.
 * @throws when none of the known locations exist, rather than letting puppeteer
 *   fail later with a less obvious message.
 */
async function findChrome() {
  const { access } = await import('node:fs/promises')
  for (const path of CHROME_CANDIDATES) {
    try {
      await access(path)
      return path
    } catch {
      /* try the next one */
    }
  }
  throw new Error(`no Chrome found; looked in:\n  ${CHROME_CANDIDATES.join('\n  ')}`)
}

/**
 * Wait for the app to be idle: fixtures resolve on a timer, so a screenshot
 * taken on `load` catches skeletons instead of data.
 *
 * @param page - `Page` — the puppeteer page.
 * @param selector - `string` — something that only exists once data has landed.
 * @returns `Promise<void>`
 */
async function settled(page, selector) {
  await page.waitForSelector(selector, { timeout: 15_000 })
  // One extra frame so CSS transitions (row hover, tab underline) have finished.
  await new Promise((r) => setTimeout(r, 350))
}

/**
 * Reset persisted UI state so every run starts from the same place.
 *
 * The stores persist to localStorage, so without this a previous run's open
 * tabs and column widths leak into the next set of images.
 *
 * @param page - `Page` — the puppeteer page.
 * @returns `Promise<void>`
 */
async function resetState(page) {
  await page.evaluateOnNewDocument(() => window.localStorage.clear())
}

/**
 * One screenshot: navigate, drive the UI into position, capture.
 *
 * @param page - `Page` — the puppeteer page.
 * @param name - `string` — output filename stem, becomes `<name>.png`.
 * @param drive - `(page: Page) => Promise<void>` — actions to reach the state.
 * @returns `Promise<void>`
 */
async function shot(page, name, drive) {
  await page.goto(URL, { waitUntil: 'domcontentloaded' })
  await settled(page, '.tree-row')
  await drive(page)
  const path = join(OUT, `${name}.png`)
  await page.screenshot({ path })
  console.log(`  ${name}.png`)
}

/**
 * Click the first element whose text matches, within a selector.
 *
 * Prefers an exact match and falls back to a substring one, because several
 * buttons carry a leading glyph in their label (`▶ Run`, `⇊ Save`) that is
 * presentational and would otherwise have to be duplicated in every call.
 *
 * @param page - `Page` — the puppeteer page.
 * @param selector - `string` — CSS selector to search within.
 * @param text - `string` — the visible label to match.
 * @returns `Promise<void>`
 * @throws when nothing matches, naming the selector and text, so a renamed
 *   button fails loudly rather than producing a screenshot of the wrong state.
 */
async function clickText(page, selector, text) {
  const handle = await page.evaluateHandle(
    (sel, txt) => {
      const all = [...document.querySelectorAll(sel)]
      return (
        all.find((e) => e.textContent?.trim() === txt) ??
        all.find((e) => e.textContent?.trim().includes(txt))
      )
    },
    selector,
    text,
  )
  const el = handle.asElement()
  if (!el) throw new Error(`no ${selector} with text ${JSON.stringify(text)}`)
  await el.click()
}

/**
 * Collapse the psql pane to its 28px bar.
 *
 * Used for the shots that are about the editor: the pane is docked open by
 * default, and an empty terminal below a feature reads as unfinished rather
 * than as a deliberate part of the layout.
 *
 * @param page - `Page` — the puppeteer page.
 * @returns `Promise<void>`
 */
async function collapseTerminal(page) {
  await clickText(page, '.term__action', 'collapse')
  await new Promise((r) => setTimeout(r, 200))
}

async function main() {
  await mkdir(OUT, { recursive: true })
  const browser = await puppeteer.launch({
    executablePath: await findChrome(),
    headless: 'new',
    defaultViewport: VIEWPORT,
    args: ['--hide-scrollbars', '--force-color-profile=srgb'],
  })

  const page = await browser.newPage()
  await resetState(page)
  console.log(`capturing from ${URL} →`)

  await shot(page, 'data-tab', async (p) => {
    // `events` is the table the design uses and the one with the richest row
    // shape; the default selection is alphabetical.
    await clickText(p, '.tree-label', 'events')
    await settled(p, '.grid-row')
  })

  await shot(page, 'relationships', async (p) => {
    await collapseTerminal(p)
    await clickText(p, '.tab', 'Relationships')
    await settled(p, '.er-card__header')
  })

  await shot(page, 'query-editor', async (p) => {
    await collapseTerminal(p)
    await clickText(p, 'button', '+ New query')
    await settled(p, '.cm-editor')
    await p.keyboard.type('select event_name, count(*) from events group by 1 order by 2 desc;')
    await clickText(p, 'button', 'Run')
    await settled(p, '.grid-row')
  })

  await shot(page, 'query-plan', async (p) => {
    await collapseTerminal(p)
    await clickText(p, 'button', '+ New query')
    await settled(p, '.cm-editor')
    await p.keyboard.type('select u.plan, count(*) from events e join users u using (user_id) group by 1;')
    await clickText(p, 'button', 'Analyze')
    await settled(p, '.plan-card__head')
  })

  await shot(page, 'psql-pane', async (p) => {
    await p.click('.term__body')
    await p.keyboard.type('select event_name, count(*) from events group by 1 order by 2 desc;')
    await p.keyboard.press('Enter')
    await settled(p, '.seg--body')
    await p.keyboard.type('\\dt')
    await p.keyboard.press('Enter')
    await new Promise((r) => setTimeout(r, 500))
  })

  await browser.close()
  console.log(`done → ${OUT}`)
}

main().catch((e) => {
  console.error(e.message)
  process.exit(1)
})
