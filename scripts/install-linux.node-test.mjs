import assert from 'node:assert/strict';
import { before, after, test } from 'node:test';
import { mkdtemp, mkdir, copyFile, readFile, writeFile, readdir, stat, symlink, rm, realpath, chmod } from 'node:fs/promises';
import { chmodSync, copyFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createInterface } from 'node:readline';

const available = process.platform === 'linux' && spawnSync('rustc', ['--version']).status === 0;
let root, publisher, oldImage, newImage, newerImage, exchangeFailure;
let stageCounter = 0;
function stage(source, directory) {
  const staged = join(directory, `AskHuman.new.${process.pid}.${stageCounter++}`);
  copyFileSync(source, staged);
  chmodSync(staged, 0o755);
  const hash = createHash('sha256').update(readFileSync(staged)).digest('hex');
  return { staged, hash };
}
const run = (source, directory, env = {}) => {
  const { staged, hash } = stage(source, directory);
  try {
    return spawnSync(publisher, [
      'publish', staged, join(directory, 'AskHuman'), join(directory, '.askhuman-install-state'), hash, hash,
    ], { encoding: 'utf8', env: { ...process.env, ...env } });
  } finally {
    rmSync(staged, { force: true });
  }
};
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
  for (const version of ['old', 'new', 'newer']) {
    const source = join(root, `${version}.rs`);
    await writeFile(source, `#[path=${JSON.stringify(resolve('src-tauri/src/executable.rs'))}] mod executable;
use std::io::{self, BufRead, Write};
fn main() {
 executable::capture_launch_path();
 let mode=std::env::args().nth(1).unwrap_or_default();
 if mode=="child" { println!("${version}"); return; }
 if ["--popup", "--popup-host", "--gui-host"].contains(&mode.as_str()) {
  println!("${version}"); println!("launch={}",executable::launch_path().unwrap().display());
  println!("context-cleared={}",std::env::var_os(executable::INTERNAL_LAUNCH_ENTRY_ENV).is_none());
  let mut command=if mode=="--gui-host" {
   let entry=executable::launch_path().unwrap();
   let mut command=std::process::Command::new(&entry);
   executable::set_launch_entry(&mut command,&entry); command.arg("child"); command
  } else {
   let entry=executable::launch_path().unwrap();
   let mut command=std::process::Command::new(&entry);
   command.arg("--gui-host"); executable::set_launch_entry(&mut command,&entry); command
  };
  let result=command.output().unwrap(); assert!(result.status.success());
  print!("{}",String::from_utf8(result.stdout).unwrap()); return;
 }
 let initial=std::fs::read(std::env::current_exe().unwrap()).unwrap();
 println!("READY"); io::stdout().flush().unwrap();
 io::stdin().lock().lines().next().unwrap().unwrap();
 let running=std::env::current_exe().unwrap();
 let launch=executable::launch_path().unwrap();
 if mode=="retained-chain" {
  println!("disk-changed={}",std::fs::read(&launch).unwrap()!=initial);
  println!("identity-unchanged={}",std::fs::read(&running).unwrap()==initial);
  for args in [vec!["--popup-host"],vec!["--popup"],vec!["--popup","--warm"]] {
   println!("role={}",args.join(" "));
   let mut command=std::process::Command::new(&running); command.args(args);
   executable::inherit_launch_entry(&mut command).unwrap();
   let result=command.output().unwrap(); assert!(result.status.success());
   print!("{}",String::from_utf8(result.stdout).unwrap());
  }
  return;
 }
 println!("{}", running.display()); println!("{}",launch.display());
 for path in [running,launch] {
  let result=std::process::Command::new(path).arg("child").output().unwrap();
  assert!(result.status.success()); print!("{}",String::from_utf8(result.stdout).unwrap());
 }
}`);
    build = spawnSync('rustc', ['--edition=2021', source, '-o', join(root, version)], { encoding: 'utf8' });
    assert.equal(build.status, 0, build.stderr);
  }
  oldImage = join(root, 'old'); newImage = join(root, 'new'); newerImage = join(root, 'newer');
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
linuxTest('retained popup, warm and GUI descendants preserve the entry across repeated upgrades', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  const canonical = join(directory, 'AskHuman');
  const child = spawn(canonical, ['retained-chain'], { cwd: '/', stdio: ['pipe', 'pipe', 'pipe'] });
  const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  try {
    assert.equal((await lines.next()).value, 'READY');
    for (const image of [newImage, newerImage]) {
      const result = run(image, directory);
      assert.equal(result.status, 0, result.stderr);
    }
    const running = await realpath(`/proc/${child.pid}/exe`);
    assert.notEqual(running, canonical);
    assert.equal(await digest(running), await digest(oldImage));
    child.stdin.end('continue\n');
    const output = [];
    for await (const line of { [Symbol.asyncIterator]: () => lines }) { output.push(line); }
    const popup = ['old', `launch=${canonical}`, 'context-cleared=true'];
    const independentGui = ['newer', `launch=${canonical}`, 'context-cleared=true'];
    assert.deepEqual(output, [
      'disk-changed=true', 'identity-unchanged=true',
      ...['--popup-host', '--popup', '--popup --warm'].flatMap(role => [
        `role=${role}`, ...popup, ...independentGui, 'newer',
      ]),
    ]);
    assert.equal(await new Promise(resolve => child.exitCode !== null ? resolve(child.exitCode) : child.once('exit', resolve)), 0);
  } finally { if (child.exitCode === null) { child.kill(); } }
});
linuxTest('unchanged install preserves inode; repeated upgrades keep only the newest retained executable', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  const first = await stat(join(directory, 'AskHuman'));
  assert.equal(run(oldImage, directory).status, 0);
  assert.equal((await stat(join(directory, 'AskHuman'))).ino, first.ino);
  assert.equal((await retained(directory)).length, 0);
  assert.equal(run(newImage, directory).status, 0);
  assert.equal(run(oldImage, directory).status, 0);
  const backups = await retained(directory);
  assert.equal(backups.length, 1);
  assert.equal(await digest(join(directory, backups[0], 'AskHuman')), await digest(newImage));
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
linuxTest('failed staged artifact hashing leaves installed executable and state unchanged', async () => {
  const directory = await fixture(); assert.equal(run(oldImage, directory).status, 0);
  const state = await readFile(join(directory, '.askhuman-install-state'), 'utf8');
  const missing = join(directory, 'missing-stage');
  const result = spawnSync(publisher, [
    'publish', missing, join(directory, 'AskHuman'), join(directory, '.askhuman-install-state'),
    '0'.repeat(64), '0'.repeat(64),
  ], { encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.equal(await digest(join(directory, 'AskHuman')), await digest(oldImage));
  assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), state);
});
linuxTest('a staged artifact with a different build hash fails before publication', async () => {
  const directory = await fixture(); assert.equal(run(oldImage, directory).status, 0);
  const state = await readFile(join(directory, '.askhuman-install-state'), 'utf8');
  const { staged, hash } = stage(newImage, directory);
  const result = spawnSync(publisher, [
    'publish', staged, join(directory, 'AskHuman'), join(directory, '.askhuman-install-state'),
    '0'.repeat(64), hash,
  ], { encoding: 'utf8' });
  rmSync(staged, { force: true });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /build artifact changed while staging/);
  assert.equal(await digest(join(directory, 'AskHuman')), await digest(oldImage));
  assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), state);
  assert.equal((await retained(directory)).length, 0);
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
  assert.equal((await retained(directory)).length, 1);
});
linuxTest('concurrent installers serialize executable and matching state publication', async () => {
  const directory = await fixture();
  const stagedFiles = Array.from({ length: 8 }, (_, index) => stage(index % 2 ? oldImage : newImage, directory));
  try {
    const results = await Promise.all(stagedFiles.map(({ staged, hash }) => new Promise(resolve => {
      const child = spawn(publisher, [
        'publish', staged, join(directory, 'AskHuman'), join(directory, '.askhuman-install-state'), hash, hash,
      ]);
      let error = ''; child.stderr.on('data', data => { error += data; });
      child.once('exit', code => resolve({ code, error }));
    })));
    for (const result of results) { assert.equal(result.code, 0, result.error); }
    const installed = await digest(join(directory, 'AskHuman'));
    assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), `source=${installed}\ninstalled=${installed}\n`);
    assert.ok((await retained(directory)).length <= 1);
  } finally {
    for (const { staged } of stagedFiles) { rmSync(staged, { force: true }); }
  }
});

linuxTest('unsupported atomic exchange fails closed and removes only unpublished staging files', async () => {
  const directory = await fixture(); assert.equal(run(oldImage, directory).status, 0);
  const state = await readFile(join(directory, '.askhuman-install-state'), 'utf8');
  const { staged, hash } = stage(newImage, directory);
  const result = spawnSync(publisher, [
    'publish', staged, join(directory, 'AskHuman'), join(directory, '.askhuman-install-state'), hash, hash,
  ], {
    encoding: 'utf8', env: { ...process.env, LD_PRELOAD: exchangeFailure },
  });
  rmSync(staged, { force: true });
  assert.equal(result.status, 1); assert.match(result.stderr, /not implemented/);
  assert.equal(await digest(join(directory, 'AskHuman')), await digest(oldImage));
  assert.equal(await readFile(join(directory, '.askhuman-install-state'), 'utf8'), state);
  assert.equal((await retained(directory)).length, 0);
});

linuxTest('keeps an image while a live process executes it and reclaims it afterwards', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  const child = spawn(join(directory, 'AskHuman'), [], { cwd: '/', stdio: ['pipe', 'pipe', 'pipe'] });
  const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  try {
    assert.equal((await lines.next()).value, 'READY');
    assert.equal(run(newImage, directory).status, 0);
    const inUse = run(oldImage, directory);
    assert.equal(inUse.status, 0, inUse.stderr);
    assert.match(inUse.stdout, new RegExp(`in use by pid ${child.pid}`));
    assert.equal((await retained(directory)).length, 2);
    child.stdin.end('continue\n');
    await new Promise(resolve => child.exitCode !== null ? resolve() : child.once('exit', resolve));
    const reclaimed = run(newImage, directory);
    assert.equal(reclaimed.status, 0, reclaimed.stderr);
    assert.match(reclaimed.stdout, /removed unreferenced retained executable/);
    assert.equal((await retained(directory)).length, 1);
  } finally { if (child.exitCode === null) { child.kill(); } }
});

linuxTest('pruning needs a complete process scan and later reclaims the held image', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  assert.equal(run(newImage, directory).status, 0);
  const [oldName] = await retained(directory);
  const oldRetained = join(directory, oldName);

  const fake = join(root, 'proc-with-reference');
  await mkdir(join(fake, '4242'), { recursive: true });
  await symlink(join(oldRetained, 'AskHuman'), join(fake, '4242', 'exe'));
  const referenced = run(newerImage, directory, { ASKHUMAN_INSTALL_PROC_ROOT: fake });
  assert.equal(referenced.status, 0, referenced.stderr);
  assert.match(referenced.stdout, /in use by pid 4242/);
  assert.equal((await retained(directory)).length, 2);

  const incomplete = run(oldImage, directory, { ASKHUMAN_INSTALL_PROC_ROOT: join(root, 'proc-missing') });
  assert.equal(incomplete.status, 0, incomplete.stderr);
  assert.match(incomplete.stderr, /retained executable pruning skipped: cannot inspect/);
  assert.match(incomplete.stdout, /cannot verify process references/);
  assert.equal((await retained(directory)).length, 3);

  const empty = join(root, 'proc-empty');
  await mkdir(empty);
  const reclaimed = run(newImage, directory, { ASKHUMAN_INSTALL_PROC_ROOT: empty });
  assert.equal(reclaimed.status, 0, reclaimed.stderr);
  assert.match(reclaimed.stdout, /removed unreferenced retained executable/);
  const left = await retained(directory);
  assert.equal(left.length, 1);
  assert.equal(await digest(join(directory, left[0], 'AskHuman')), await digest(oldImage));
});

linuxTest('an unchanged install also reclaims unreferenced images', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  assert.equal(run(newImage, directory).status, 0);
  const [oldName] = await retained(directory);
  const fake = join(root, 'proc-hold-unchanged');
  await mkdir(join(fake, '4343'), { recursive: true });
  await symlink(join(directory, oldName, 'AskHuman'), join(fake, '4343', 'exe'));
  assert.equal(run(newerImage, directory, { ASKHUMAN_INSTALL_PROC_ROOT: fake }).status, 0);
  assert.equal((await retained(directory)).length, 2);

  const unchanged = run(newerImage, directory);
  assert.equal(unchanged.status, 0, unchanged.stderr);
  assert.match(unchanged.stdout, /installed executable unchanged/);
  assert.match(unchanged.stdout, /removed unreferenced retained executable/);
  assert.equal((await retained(directory)).length, 1);
});

linuxTest('an unreadable exe blocks pruning only when the process could be AskHuman', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  assert.equal(run(newImage, directory).status, 0);
  const [oldName] = await retained(directory);
  const held = join(root, 'proc-hold-exe');
  await mkdir(join(held, '4444'), { recursive: true });
  await symlink(join(directory, oldName, 'AskHuman'), join(held, '4444', 'exe'));
  assert.equal(run(newerImage, directory, { ASKHUMAN_INSTALL_PROC_ROOT: held }).status, 0);
  assert.equal((await retained(directory)).length, 2);

  const askhuman = join(root, 'proc-loop-askhuman');
  await mkdir(join(askhuman, '5151'), { recursive: true });
  await symlink('exe', join(askhuman, '5151', 'exe'));
  await writeFile(join(askhuman, '5151', 'comm'), 'AskHuman\n');
  const blocked = run(oldImage, directory, { ASKHUMAN_INSTALL_PROC_ROOT: askhuman });
  assert.equal(blocked.status, 0, blocked.stderr);
  assert.match(blocked.stderr, /process is named AskHuman/);
  assert.equal((await retained(directory)).length, 3);

  const other = join(root, 'proc-loop-other');
  await mkdir(join(other, '5252'), { recursive: true });
  await symlink('exe', join(other, '5252', 'exe'));
  await writeFile(join(other, '5252', 'comm'), 'bash\n');
  const reclaimed = run(newImage, directory, { ASKHUMAN_INSTALL_PROC_ROOT: other });
  assert.equal(reclaimed.status, 0, reclaimed.stderr);
  assert.match(reclaimed.stdout, /removed unreferenced retained executable/);
  const left = await retained(directory);
  assert.equal(left.length, 1);
  assert.equal(await digest(join(directory, left[0], 'AskHuman')), await digest(oldImage));
});

const nonRootLinuxTest = (name, fn) => test(name, {
  skip: !available ? 'requires Linux and the Rust toolchain'
    : process.getuid?.() === 0 ? 'requires a non-root user' : false,
}, fn);

nonRootLinuxTest('a failed reclamation keeps the image and reports a diagnostic', async () => {
  const directory = await fixture();
  assert.equal(run(oldImage, directory).status, 0);
  assert.equal(run(newImage, directory).status, 0);
  const [oldName] = await retained(directory);
  const oldDirectory = join(directory, oldName);
  await chmod(oldDirectory, 0o555);
  try {
    const result = run(newerImage, directory);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /cannot remove: Permission denied/);
    assert.ok((await retained(directory)).includes(oldName));
  } finally { await chmod(oldDirectory, 0o755); }
});
