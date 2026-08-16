/* tslint:disable */
/* eslint-disable */
export function create_wallet(birth_height: number): any;
export function restore_wallet(phrase: string, birth_height: number): any;
export function wallet_address(state: string): string;
export function wallet_scan_from(state: string): number;
export function ingest_page(state: string, outputs_json: string, spent_json: string, scanned_to: number): any;
export function wallet_balance(state: string, tip: number): any;
export function wallet_history(state: string): any;
export function build_send(state: string, to: string, amount: string, memo: string, tip: number): any;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly create_wallet: (a: number) => [number, number, number];
  readonly restore_wallet: (a: number, b: number, c: number) => [number, number, number];
  readonly wallet_address: (a: number, b: number) => [number, number, number, number];
  readonly wallet_scan_from: (a: number, b: number) => [number, number, number];
  readonly ingest_page: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number];
  readonly wallet_balance: (a: number, b: number, c: number) => [number, number, number];
  readonly wallet_history: (a: number, b: number) => [number, number, number];
  readonly build_send: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => [number, number, number];
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __externref_table_alloc: () => number;
  readonly __wbindgen_export_2: WebAssembly.Table;
  readonly __externref_table_dealloc: (a: number) => void;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
