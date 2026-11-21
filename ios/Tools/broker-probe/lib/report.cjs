'use strict'

// Turns the suite's results into text, JSON or a Markdown table for the docs.

const { CHECKS } = require('./checks.cjs')

const WORD = { ok: 'ok  ', fail: 'FAIL', warn: 'warn', info: 'info', skip: 'skip' }
const MARK = { ok: '✅', fail: '❌', warn: '⚠️', info: 'ℹ️', skip: '⏭' }

/** One line per result, in the order the checks finish. */
function line(result) {
  const where = (result.transport || 'all').padEnd(3)
  return `${WORD[result.status]}  ${where}  ${result.id.padEnd(13)} ${result.note}`
}

function summarize(results) {
  const count = (status) => results.filter((r) => r.status === status).length
  const mustFailed = results.filter((r) => r.status === 'fail' && r.level === 'must')
  return {
    ok: count('ok'),
    fail: count('fail'),
    warn: count('warn'),
    info: count('info'),
    skip: count('skip'),
    pass: results.filter((r) => r.status === 'fail').length === 0,
    mustFailed: mustFailed.map((r) => r.id),
  }
}

/** The closing block of the text report. `options.only` marks a partial run, which can never say "compatible". */
function textSummary(results, options = {}) {
  const s = summarize(results)
  const out = ['']
  const failed = results.filter((r) => r.status === 'fail')
  const warned = results.filter((r) => r.status === 'warn')
  if (failed.length > 0) {
    out.push('Required checks that failed:')
    for (const r of failed) out.push(`  - [${r.transport || 'all'}] ${r.title}: ${r.note}`)
  }
  if (warned.length > 0) {
    out.push('Warnings (the channel works, but a safeguard is missing or the broker differs in a harmless way):')
    for (const r of warned) out.push(`  - [${r.transport || 'all'}] ${r.title}: ${r.note}`)
  }
  const clean = options.strict ? s.pass && s.warn === 0 : s.pass
  let verdict
  if (options.only) verdict = clean ? `the ${options.only} checks passed` : `the ${options.only} checks did not pass`
  else if (!s.pass) verdict = 'NOT COMPATIBLE'
  else if (s.warn === 0) verdict = 'COMPATIBLE'
  else verdict = options.strict ? 'NOT COMPATIBLE (--strict counts warnings as failures)' : 'COMPATIBLE (with warnings)'
  out.push(`${s.ok} ok, ${s.fail} failed, ${s.warn} warnings, ${s.info} measurements, ${s.skip} skipped  ->  ${verdict}`)
  return out.join('\n')
}

function toJson(meta, results) {
  return JSON.stringify({ ...meta, summary: summarize(results), results }, null, 2)
}

const cell = (result) => (result ? `${MARK[result.status]} ${result.note.replace(/\|/g, '\\|')}` : '')

/** A Markdown table: one row per check, one column per transport (checks that run once get a single cell). */
function toMarkdown(meta, results) {
  const kinds = [...new Set(results.map((r) => r.transport).filter(Boolean))]
  const ids = Object.keys(CHECKS).filter((id) => results.some((r) => r.id === id))
  const head = `| Check | Level | ${kinds.join(' | ')} |`
  const rule = `| --- | --- | ${kinds.map(() => '---').join(' | ')} |`
  const rows = ids.map((id) => {
    const cells = kinds.map((kind) => cell(results.find((r) => r.id === id && r.transport === kind)))
    return `| ${CHECKS[id].title.replace(/\|/g, '\\|')} | ${CHECKS[id].level} | ${cells.join(' | ')} |`
  })
  const when = meta.date
  const title = `Compatibility run: ${meta.label} (${when}, node ${meta.node})`
  return [`**${title}**`, '', head, rule, ...rows].join('\n')
}

module.exports = { line, summarize, textSummary, toJson, toMarkdown }
