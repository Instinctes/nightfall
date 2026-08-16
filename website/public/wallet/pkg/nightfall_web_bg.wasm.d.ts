/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export const wasm_start: () => void;
export const create_wallet: (a: number) => [number, number, number];
export const restore_wallet: (a: number, b: number, c: number) => [number, number, number];
export const wallet_address: (a: number, b: number) => [number, number, number, number];
export const wallet_phrase: (a: number, b: number) => [number, number, number, number];
export const wallet_view_key: (a: number, b: number) => [number, number, number, number];
export const wallet_scan_from: (a: number, b: number) => [number, number, number];
export const wallet_info: (a: number, b: number) => [number, number, number];
export const reset_scan: (a: number, b: number) => [number, number, number];
export const address_qr_svg: (a: number, b: number) => [number, number, number, number];
export const ingest_page: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number];
export const wallet_balance: (a: number, b: number, c: number) => [number, number, number];
export const wallet_history: (a: number, b: number) => [number, number, number];
export const probe_crypto: () => [number, number, number, number];
export const build_send: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => [number, number, number];
export const __wbindgen_exn_store: (a: number) => void;
export const __externref_table_alloc: () => number;
export const __wbindgen_export_2: WebAssembly.Table;
export const __wbindgen_free: (a: number, b: number, c: number) => void;
export const __wbindgen_malloc: (a: number, b: number) => number;
export const __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
export const __externref_table_dealloc: (a: number) => void;
export const __wbindgen_start: () => void;
