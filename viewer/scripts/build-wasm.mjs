// Builds the viewer's renderer, crates/agentdrafter-viewer, for WebAssembly
// and writes the module with its JavaScript glue into src/wasm/, where the
// page imports it. Needs the Rust toolchain with the wasm32-unknown-unknown
// target, and wasm-bindgen-cli of the version the crate pins.
//
// CARGO_BUILD_JOBS limits Cargo's jobs as usual; WASM_BINDGEN names the
// wasm-bindgen binary when it is not on the PATH.
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const viewer = dirname(dirname(fileURLToPath(import.meta.url)));
const root = dirname(viewer);
const out = join(viewer, 'src', 'wasm');
const manifest = readFileSync(join(root, 'crates', 'agentdrafter-viewer', 'Cargo.toml'), 'utf8');
const pinned = /wasm-bindgen = "=([0-9.]+)"/.exec(manifest)?.[1];
if (!pinned) throw new Error('crates/agentdrafter-viewer/Cargo.toml pins no wasm-bindgen version');

const run = (command, args) => execFileSync(command, args, { cwd: root, stdio: 'inherit' });
const bindgen = process.env.WASM_BINDGEN || 'wasm-bindgen';
let version;
try {
  version = execFileSync(bindgen, ['--version'], { encoding: 'utf8' }).trim().split(' ').pop();
} catch {
  throw new Error(`wasm-bindgen-cli ${pinned} is needed: cargo install wasm-bindgen-cli --version ${pinned} --locked`);
}
if (version !== pinned) {
  throw new Error(`wasm-bindgen-cli is ${version}; the crate pins ${pinned}: cargo install wasm-bindgen-cli --version ${pinned} --locked`);
}

run('cargo', ['build', '--locked', '-p', 'agentdrafter-viewer', '--target', 'wasm32-unknown-unknown', '--profile', 'wasm-release']);
const target = process.env.CARGO_TARGET_DIR || join(root, 'target');
const module = join(target, 'wasm32-unknown-unknown', 'wasm-release', 'agentdrafter_viewer.wasm');
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
run(bindgen, ['--target', 'web', '--out-dir', out, module]);
