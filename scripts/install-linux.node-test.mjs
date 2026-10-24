import assert from 'node:assert/strict';
import { before, after, test } from 'node:test';
import { mkdtemp, mkdir, copyFile, readFile, writeFile, readdir, stat, symlink, rm, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createInterface } from 'node:readline';

const available = process.platform === 'linux' && spawnSync('rustc', ['--version']).status === 0;
let root, publisher, oldImage, newImage, exchangeFailure;
const run = (source, directory) => spawnSync(publisher, [source, directory], { encoding: 'utf8' });
const digest = async path => createHash('sha256').update(await readFile(path)).digest('hex');
const cases = [];
async function fixture() {
  const directory = await mkdtemp(join(root, 'target with spaces-'));
  cases.push(directory);
  return directory;
}
const retained = async directory => (await readdir(directory)).filter(name => /^\.askhuman-install-\d+-\d+$/.test(name));
before(async () => {
  if (!available) { return; }
  root = await mkdtemp(join(tmpdir(), 'askhuman-install-test-'));
  publisher = join(root, 'publisher');
  let build = spawnSync('rustc', ['--edition=2021', '-Dwarnings', 'scripts/install-linux.rs', '-o', publisher], { encoding: 'utf8' });
  assert.equal(build.status, 0, build.stderr);
  for (const version of ['old', 'new']) {
    const source = join(root, `${version}.rs`);
    await writeFile(source, `#[path=${JSON.stringify(resolve('src-tauri/src/executable.rs'))}] mod executable;
use std::io::{self, BufRead, Write};
fn main() {
 executable::capture_launch_path();
 if std::env::args().nth(1).as_deref()==Some("child") { println!("${version}"); return; }
 println!("READY"); io::stdout().flush().unwrap();
 io::stdin().lock().lines().next().unwrap().unwrap();
 let running=std::env::current_exe().unwrap();
 let launch=executable::launch_path().unwrap();
 println!("{}", running.display()); println!("{}",launch.display());
 for path in [running,launch] {
  let result=std::process::Command::new(path).arg("child").output().unwrap();
  assert!(result.status.success()); print!("{}",String::from_utf8(result.stdout).unwrap());
 }
}`);
    build = spawnSync('rustc', ['--edition=2021', source, '-o', join(root, version)], { encoding: 'utf8' });
    assert.equal(build.status, 0, build.stderr);
  }
  oldImage = join(root, 'old'); newImage = join(root, 'new');
  exchangeFailure = join(root, 'exchange-failure.so');
  const failureSource = join(root, 'exchange-failure.rs');
  await writeFile(failureSource, `unsafe extern "C" { fn __errno_location() -> *mut i32; }
#[no_mangle] pub extern "C" fn renameat2(_:i32,_:*const u8,_:i32,_:*const u8,_:u32)->i32 {
 unsafe { *__errno_location()=38; } -1
}`);
  build = spawnSync('rustc', ['--edition=2021', '--crate-type=cdylib', failureSource, '-o', exchangeFailure], { encoding: 'utf8' });
  assert.equal(build.status, 0, build.stderr);
});
after(async () => { if (root) { await rm(root, { recursive: true, force: true }); } });
const linuxTest = (name, fn) => test(name, { skip: !available && 'requires Linux and the Rust toolchain' }, fn);

linuxTest('running ELF survives exchange and cached launch path starts the new image outside Dev cwd', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  const canonical = join(directory, 'AskHuman');
  const child = spawn(canonical, [], { cwd: '/', stdio: ['pipe', 'pipe', 'pipe'] });
  const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  try {
    assert.equal((await lines.next()).value, 'READY');
    const result = run(newImage, directory);
    assert.equal(result.status, 0, result.stderr);
    const backups = await retained(directory);
    assert.equal(backups.length, 1);
    const oldPath = join(directory, backups[0], 'AskHuman');
    assert.equal(await realpath(`/proc/${child.pid}/exe`), oldPath);
    assert.equal(await digest(oldPath), await digest(oldImage));
    child.stdin.end('continue\n');
    const output = [];
    for await (const line of { [Symbol.asyncIterator]: () => lines }) { output.push(line); }
    assert.deepEqual(output, [oldPath, canonical, 'old', 'new']);
    assert.equal(await new Promise(resolve => child.exitCode !== null ? resolve(child.exitCode) : child.once('exit', resolve)), 0);
  } finally { if (child.exitCode === null) { child.kill(); } }
});
linuxTest('unchanged install preserves inode; repeated upgrades retain unique executable paths', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  const first = await stat(join(directory, 'AskHuman'));
  assert.equal(run(oldImage, directory).status, 0);
  assert.equal((await stat(join(directory, 'AskHuman'))).ino, first.ino);
  assert.equal((await retained(directory)).length, 0);
  assert.equal(run(newImage, directory).status, 0);
  assert.equal(run(oldImage, directory).status, 0);
  const backups = await retained(directory);
  assert.equal(backups.length, 2);
  assert.deepEqual(new Set(await Promise.all(backups.map(name => digest(join(directory, name, 'AskHuman'))))), new Set([await digest(oldImage), await digest(newImage)]));
});
linuxTest('reject symlink and directory targets before changing either target or install state', async () => {
  for (const kind of ['symlink', 'directory']) {
    const directory = await fixture();
    const destination = join(directory, 'AskHuman');
    if (kind === 'symlink') { await symlink(oldImage, destination); } else { await mkdir(destination); }
    await writeFile(join(directory, '.askhuman-install-state'), 'sentinel');
    const result = run(newImage, directory);
    assert.equal(result.status, 1); assert.match(result.stderr, /non-regular/);
    assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), 'sentinel');
    assert.equal((await retained(directory)).length, 0);
    assert.equal(await digest(oldImage), await digest(join(root, 'old')));
  }
});
linuxTest('failed source hashing leaves installed executable and state unchanged', async () => {
  const directory = await fixture(); assert.equal(run(oldImage, directory).status, 0);
  const state = await readFile(join(directory, '.askhuman-install-state'), 'utf8');
  assert.equal(run(join(root, 'missing'), directory).status, 1);
  assert.equal(await digest(join(directory, 'AskHuman')), await digest(oldImage));
  assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), state);
});
linuxTest('state failure reports successful executable publication and preserves the old image', async () => {
  const directory = await fixture(); await copyFile(oldImage, join(directory, 'AskHuman'));
  await mkdir(join(directory, '.askhuman-install-state'));
  const result = run(newImage, directory);
  assert.equal(result.status, 1); assert.match(result.stderr, /executable installed, but install-state publication failed/);
  assert.equal(await digest(join(directory, 'AskHuman')), await digest(newImage));
  const backups = await retained(directory); assert.equal(backups.length, 1);
  assert.equal(await digest(join(directory, backups[0], 'AskHuman')), await digest(oldImage));
  await rm(join(directory, '.askhuman-install-state'), { recursive: true });
  assert.equal(run(newImage, directory).status, 0);
});
linuxTest('concurrent installers serialize executable and matching state publication', async () => {
  const directory = await fixture();
  const results = await Promise.all(Array.from({ length: 8 }, (_, index) => new Promise(resolve => {
    const child = spawn(publisher, [index % 2 ? oldImage : newImage, directory]);
    let error = ''; child.stderr.on('data', data => { error += data; });
    child.once('exit', code => resolve({ code, error }));
  })));
  for (const result of results) { assert.equal(result.code, 0, result.error); }
  const installed = await digest(join(directory, 'AskHuman'));
  assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), `source=${installed}\ninstalled=${installed}\n`);
});

linuxTest('unsupported atomic exchange fails closed and removes only unpublished staging files', async () => {
  const directory = await fixture(); assert.equal(run(oldImage, directory).status, 0);
  const state = await readFile(join(directory, '.askhuman-install-state'), 'utf8');
  const result = spawnSync(publisher, [newImage, directory], {
    encoding: 'utf8', env: { ...process.env, LD_PRELOAD: exchangeFailure },
  });
  assert.equal(result.status, 1); assert.match(result.stderr, /not implemented/);
  assert.equal(await digest(join(directory, 'AskHuman')), await digest(oldImage));
  assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), state);
  assert.equal((await retained(directory)).length, 0);
});
