import { closeSync, openSync, readFileSync, unlinkSync, writeSync } from "node:fs"

/**
 * One process may drive the shared Chrome profile at a time. A second one would either fail to launch
 * or attach to the first browser, and then downloads and pages cross over. The lock is a file holding
 * a pid, and a pid that no longer runs is treated as a crash, not a holder.
 */
export type Lock = { path: string; release: () => void }

export const acquire = (path: string): Lock | "busy" => {
  for (const attempt of [0, 1]) {
    try {
      const fd = openSync(path, "wx")
      writeSync(fd, String(process.pid))
      closeSync(fd)
      return { path, release: () => release(path) }
    } catch (cause) {
      if ((cause as NodeJS.ErrnoException).code !== "EEXIST") throw cause
      const holder = Number(readFileSync(path, "utf8").trim())
      if (attempt === 0 && Number.isInteger(holder) && holder > 0 && !isRunning(holder)) {
        release(path)
        continue
      }
      return "busy"
    }
  }
  return "busy"
}

const isRunning = (pid: number): boolean => {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

const release = (path: string): void => {
  try {
    unlinkSync(path)
  } catch {
    // already gone
  }
}
