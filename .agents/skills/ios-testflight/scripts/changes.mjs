#!/usr/bin/env node
// List the iOS commits that a new TestFlight build would add on top of the previous one.
//
// The baseline is the local annotated tag `ios-testflight-N` with the largest N; it points at the
// source commit of build N. This script never talks to Apple and never reads credentials: the skill
// compares the baseline number with the builds that `status.mjs` reports.
import { execFileSync } from 'node:child_process';
import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const TAG_PATTERN = /^ios-testflight-(\d+)$/;
const FIELD = '\x1f';

export class ChangesError extends Error {
  constructor(reason, message, details = {}) {
    super(message);
    this.name = 'ChangesError';
    this.reason = reason;
    this.details = details;
  }
}

function git(cwd, args, { allowFailure = false } = {}) {
  try {
    return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  } catch (error) {
    if (allowFailure) return null;
    const stderr = String(error.stderr ?? '').trim();
    throw new ChangesError('git', `git ${args.join(' ')} failed${stderr ? `: ${stderr}` : ''}`);
  }
}

function resolveCommit(cwd, ref, label) {
  const full = git(cwd, ['rev-parse', '--verify', '--quiet', `${ref}^{commit}`], { allowFailure: true });
  if (!full || !full.trim()) throw new ChangesError('badRef', `${label} "${ref}" is not a commit in this repository.`, { ref });
  return full.trim();
}

function isAncestor(cwd, ancestor, descendant) {
  try {
    execFileSync('git', ['merge-base', '--is-ancestor', ancestor, descendant], { cwd, stdio: 'ignore' });
    return true;
  } catch (error) {
    if (error.status === 1) return false;
    throw new ChangesError('git', `git merge-base failed for ${ancestor.slice(0, 7)} and ${descendant.slice(0, 7)}.`);
  }
}

/** Conventional Commit type of a subject, or `other` when the subject does not follow the format. */
export function commitType(subject) {
  const match = /^([A-Za-z]+)(\([^)]*\))?!?:\s/.exec(subject);
  return match ? match[1].toLowerCase() : 'other';
}

function listCommits(cwd, range) {
  const out = git(cwd, ['log', '--no-merges', '--reverse', '--date=short', `--format=%H${FIELD}%h${FIELD}%ad${FIELD}%s`, range, '--', 'ios']);
  return out.split('\n').filter(Boolean).map((line) => {
    const [hash, short, date, subject] = line.split(FIELD);
    return { hash, short, date, type: commitType(subject), subject };
  });
}

function diffStat(cwd, from, to) {
  const summary = git(cwd, ['diff', '--shortstat', from, to, '--', 'ios']).trim();
  const number = (pattern) => Number(pattern.exec(summary)?.[1] ?? 0);
  return {
    files: number(/(\d+) files? changed/),
    insertions: number(/(\d+) insertions?\(\+\)/),
    deletions: number(/(\d+) deletions?\(-\)/),
    summary: summary || 'no changes under ios/',
  };
}

function baselineTag(cwd) {
  const builds = git(cwd, ['tag', '--list', 'ios-testflight-*']).split('\n')
    .map((name) => ({ name, match: TAG_PATTERN.exec(name) })).filter((entry) => entry.match)
    .map((entry) => ({ tag: entry.name, build: Number(entry.match[1]) }));
  if (builds.length === 0) {
    throw new ChangesError('noBaseline', 'No local ios-testflight-N tag exists, so the previous build\'s source commit is unknown. Ask the user; do not guess.');
  }
  const newest = builds.reduce((best, entry) => (entry.build > best.build ? entry : best));
  const commit = git(cwd, ['rev-parse', '--verify', '--quiet', `${newest.tag}^{commit}`], { allowFailure: true })?.trim();
  if (!commit) throw new ChangesError('badBaseline', `${newest.tag} does not point at a commit.`, { tag: newest.tag });
  return { ...newest, commit };
}

/**
 * @param {{cwd?: string, tip?: string, point?: string, expectDropped?: string[]}} options
 *   tip: the commit that would be built by default (HEAD). point: the commit the build is actually cut from.
 *   expectDropped: commits the human wants to leave out by moving `point` before `tip`.
 */
export function computeChanges({ cwd = process.cwd(), tip = 'HEAD', point, expectDropped } = {}) {
  const head = resolveCommit(cwd, 'HEAD', 'HEAD');
  const tipFull = resolveCommit(cwd, tip, 'Tip');
  const pointFull = point === undefined ? tipFull : resolveCommit(cwd, point, 'Build point');

  if (pointFull === head) {
    const dirty = git(cwd, ['status', '--porcelain']).split('\n').filter(Boolean);
    if (dirty.length > 0) {
      throw new ChangesError('dirtyTree', `The working tree has uncommitted changes (${dirty.length} paths); a build must come from a definite commit. Ask the user.`, { paths: dirty.slice(0, 20) });
    }
  }

  const baseline = baselineTag(cwd);
  if (!isAncestor(cwd, baseline.commit, pointFull)) {
    throw new ChangesError('notAncestor', `${baseline.tag} (${baseline.commit.slice(0, 7)}) is not an ancestor of the build point ${pointFull.slice(0, 7)}. Ask the user.`, { tag: baseline.tag });
  }

  const commits = listCommits(cwd, `${baseline.commit}..${pointFull}`);
  const result = {
    point: pointFull,
    pointShort: pointFull.slice(0, 7),
    tip: tipFull,
    baseline: { tag: baseline.tag, build: baseline.build, commit: baseline.commit },
    nextBuild: baseline.build + 1,
    commits,
    empty: commits.length === 0,
    stat: diffStat(cwd, baseline.commit, pointFull),
    dropped: [],
  };

  if (pointFull !== tipFull) {
    if (!isAncestor(cwd, pointFull, tipFull)) {
      throw new ChangesError('pointNotBeforeTip', `The build point ${pointFull.slice(0, 7)} is not an ancestor of ${tipFull.slice(0, 7)}; only an earlier point of the same history can leave commits out.`);
    }
    result.dropped = listCommits(cwd, `${pointFull}..${tipFull}`);
  }

  if (expectDropped !== undefined) {
    const expected = new Set(expectDropped.map((token) => {
      if (!/^[0-9a-f]{7,40}$/i.test(token)) throw new ChangesError('badRef', `"${token}" is not a commit hash (7-40 hex digits).`, { ref: token });
      return resolveCommit(cwd, token, 'Expected exclusion');
    }));
    const actual = new Set(result.dropped.map((commit) => commit.hash));
    const extra = result.dropped.filter((commit) => !expected.has(commit.hash));
    const missing = [...expected].filter((hash) => !actual.has(hash));
    if (extra.length > 0 || missing.length > 0) {
      throw new ChangesError('droppedMismatch',
        `The build point would drop ${extra.length} commit(s) the user did not choose to exclude and keep ${missing.length} commit(s) the user wants out. Ask the user; do not cherry-pick.`,
        { extra, missing: missing.map((hash) => hash.slice(0, 7)) });
    }
  }
  return result;
}

export function formatText(result) {
  const lines = [
    `Build point: ${result.pointShort} (${result.point})`,
    `Baseline: ${result.baseline.tag} -> ${result.baseline.commit.slice(0, 7)}`,
    `Next build number: ${result.nextBuild}`,
    result.empty ? 'iOS commits since the baseline: none' : `iOS commits since the baseline (${result.commits.length}), oldest first:`,
    ...result.commits.map((commit) => `  ${commit.short} ${commit.date} ${commit.type.padEnd(8)} ${commit.subject}`),
    `Diff under ios/: ${result.stat.summary}`,
  ];
  if (result.dropped.length > 0) {
    lines.push(`Left out by this build point (${result.dropped.length}):`, ...result.dropped.map((commit) => `  ${commit.short} ${commit.date} ${commit.type.padEnd(8)} ${commit.subject}`));
  }
  return lines.join('\n');
}

const USAGE = `Usage: node changes.mjs [--tip <commit>] [--point <commit>] [--expect-dropped <hash,hash,...>] [--json]
Lists the iOS commits (non-merge, under ios/) between the newest local ios-testflight-N tag and the build point.
  --tip              the commit that would be built by default (default: HEAD)
  --point            the commit the build is cut from (default: --tip); must be an earlier commit of the same history
  --expect-dropped   commits the user wants left out; fails unless --point drops exactly those
  --json             machine-readable output; failures are {"error":{"reason","message","details"}}
Never contacts Apple. Exit 1 when a rule fails, 2 on a usage error.`;

function parseArguments(argv) {
  const options = { json: false };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = () => {
      if (index + 1 >= argv.length) throw new Error(`${flag} needs a value`);
      index += 1;
      return argv[index];
    };
    if (flag === '--tip') options.tip = value();
    else if (flag === '--point') options.point = value();
    else if (flag === '--expect-dropped') options.expectDropped = value().split(',').map((token) => token.trim()).filter(Boolean);
    else if (flag === '--json') options.json = true;
    else if (flag === '--help' || flag === '-h') options.help = true;
    else throw new Error(`unknown argument ${flag}`);
  }
  return options;
}

function main() {
  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    console.error(`changes: ${error.message}\n${USAGE}`);
    return 2;
  }
  if (options.help) {
    console.log(USAGE);
    return 0;
  }
  try {
    const result = computeChanges({ tip: options.tip, point: options.point, expectDropped: options.expectDropped });
    console.log(options.json ? JSON.stringify(result, null, 2) : formatText(result));
    return 0;
  } catch (error) {
    if (!(error instanceof ChangesError)) throw error;
    if (options.json) console.log(JSON.stringify({ error: { reason: error.reason, message: error.message, details: error.details } }, null, 2));
    else console.error(`changes: ${error.reason}: ${error.message}`);
    return 1;
  }
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  process.exitCode = main();
}
