import type { ChildProcess } from 'node:child_process'
export declare function startServers(): Promise<{
  urls: Record<string, string>
  running: { name: string; base: string; child: ChildProcess }[]
  stop: () => void
}>
