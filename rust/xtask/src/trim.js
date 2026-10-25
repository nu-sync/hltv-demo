// Trim a page to the subtrees the extractors read, then serialize it as Playwright's page.content()
// does. Evaluated inside the page, so it may not close over anything: the same rule as the readers.
// Ported from scripts/capture-fixtures.ts, rule for rule.
(() => {
  const serialize = () =>
    (document.doctype ? new XMLSerializer().serializeToString(document.doctype) : "") +
    (document.documentElement ? document.documentElement.outerHTML : "")
  const before = serialize().length

  /** A node the extractors read: kept with its whole subtree. Everything above it survives as path. */
  const read = new Set()
  const roots = [
    ".results-sublist", // a date heading plus the rows under it
    ".result-con", // a row, including the featured block that has no heading
    ".timeAndEvent", // the match page's date and event
    ".streams", // where the demo link lives
    'a[href^="/download/demo/"]',
    "title",
  ]
  for (const selector of roots) for (const node of Array.from(document.querySelectorAll(selector))) read.add(node)

  // A read node is kept whole: the extractors read the markup inside a row, so pruning stops there.
  // An ancestor is only a path to one, so its other children go.
  const prune = (node) => {
    let hasContent = false
    for (const child of Array.from(node.children)) {
      if (read.has(child)) {
        hasContent = true
        continue
      }
      if (prune(child)) hasContent = true
      else child.remove()
    }
    return hasContent
  }
  prune(document.documentElement)

  // Nothing executable and nothing that phones home survives the prune, but be explicit: a future
  // page may put a script inside a kept subtree.
  for (const node of Array.from(
    document.querySelectorAll(
      "script, noscript, iframe, link, style, [data-cf-beacon], .pixel-tracking, img[src*='facebook.com/tr']",
    ),
  )) {
    node.remove()
  }

  // Capture metadata hides in comments ("Country: US-IL", build ids); comments are not structure.
  const dropComments = (node) => {
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType === Node.COMMENT_NODE) child.parentNode?.removeChild(child)
      else dropComments(child)
    }
  }
  dropComments(document)

  // Only structure survives, by explicit allow-list: no per-visitor attributes (one reader's country,
  // ad config), no inline style, no handlers. A new selector that needs another attribute fails the
  // fixture test, which is the point of having one.
  const STRUCTURAL =
    /^(class|href|src|alt|title|id|lang|colspan|rowspan|type|rel|charset|data-unix|data-demo-link|data-time-format)$/
  for (const node of [document.documentElement, ...Array.from(document.querySelectorAll("*"))]) {
    for (const attribute of Array.from(node.attributes)) {
      if (!STRUCTURAL.test(attribute.name)) node.removeAttribute(attribute.name)
    }
  }

  if (document.querySelector("meta[charset]") === null) {
    const meta = document.createElement("meta")
    meta.setAttribute("charset", "utf-8")
    document.head.prepend(meta)
  }

  return { before, html: serialize() }
})()
