export type Device = { id: string; name: string; model: string; online: boolean; channels: number }
export type Scenario = 'empty' | 'one' | 'many' | 'grid' | 'settings' | 'desktop'
export declare const SCENARIOS: Record<Scenario, Device[]>
export declare const BROWSERS: readonly string[]
export declare const PER_BROWSER: readonly Scenario[]
export declare const SERVERS_FILE: string
export declare function serverNames(): string[]
