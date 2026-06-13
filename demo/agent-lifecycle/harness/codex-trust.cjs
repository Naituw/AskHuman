'use strict';

// Compute Codex hook "trust" entries for a project hooks.json, reproducing
// Codex's own algorithm from source (codex-rs):
//
//   state key  = "<abs hooks.json path>:<event_label>:<group_index>:<handler_index>"
//                (hooks/src/lib.rs::hook_key; key_source = hooks.json abs path)
//   trusted_hash = version_for_toml(NormalizedHookIdentity)
//                = "sha256:" + sha256hex( compact( canonical_sort( json(identity) ) ) )
//                (config/src/fingerprint.rs::version_for_toml + canonical_json)
//
//   NormalizedHookIdentity = { event_name: <label>, <flattened MatcherGroup> }
//   MatcherGroup           = { matcher?: <resolved>, hooks: [ normalized_handler ] }
//   normalized_handler     = { type:"command", command, timeout: <sec>, async: <bool>, statusMessage? }
//     - timeout defaults to 600, min 1   (discovery.rs)
//     - matcher: UserPromptSubmit/Stop force None; others keep as-is (events/common.rs)
//     - command_windows dropped on non-Windows; None fields omitted by toml serializer
//
// 该 trusted_hash 必须写进 **用户级** ~/.codex/config.toml 的 [hooks.state."<key>"]
// （config_rules.rs：hook state 只从 User / SessionFlags 层读取，项目层不算数）。
//
// 用法: node codex-trust.cjs <abs-or-rel path to .codex/hooks.json>
//   打印可直接追加到 ~/.codex/config.toml 的 TOML 片段。

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const EVENT_LABEL = {
  PreToolUse: 'pre_tool_use',
  PermissionRequest: 'permission_request',
  PostToolUse: 'post_tool_use',
  PreCompact: 'pre_compact',
  PostCompact: 'post_compact',
  SessionStart: 'session_start',
  UserPromptSubmit: 'user_prompt_submit',
  SubagentStart: 'subagent_start',
  SubagentStop: 'subagent_stop',
  Stop: 'stop',
};
// 这两个事件不支持 matcher（events/common.rs::matcher_pattern_for_event 强制 None）。
const FORCE_NO_MATCHER = new Set(['user_prompt_submit', 'stop']);

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') {
    const out = {};
    for (const k of Object.keys(value).sort()) out[k] = canonical(value[k]);
    return out;
  }
  return value;
}

function versionForToml(identity) {
  // serde_json::to_vec(canonical) 是紧凑无空格 JSON；JS JSON.stringify 默认一致。
  const serialized = JSON.stringify(canonical(identity));
  const hex = crypto.createHash('sha256').update(serialized, 'utf8').digest('hex');
  return `sha256:${hex}`;
}

function normalizedHandler(handler) {
  // 非 Windows：用 command（commandWindows 仅 cfg!(windows) 时替换）。
  const out = {
    type: 'command',
    command: handler.command,
    timeout: Math.max(1, handler.timeout != null ? handler.timeout : 600),
    async: handler.async === true,
  };
  if (handler.statusMessage != null) out.statusMessage = handler.statusMessage;
  return out;
}

function computeEntries(hooksJsonPath) {
  const abs = path.resolve(hooksJsonPath);
  const doc = JSON.parse(fs.readFileSync(abs, 'utf8'));
  const events = (doc && doc.hooks) || {};
  const entries = [];
  for (const [eventName, groups] of Object.entries(events)) {
    const label = EVENT_LABEL[eventName];
    if (!label) {
      entries.push({ warning: `unknown event "${eventName}" (skipped)` });
      continue;
    }
    (groups || []).forEach((group, groupIndex) => {
      let matcher = group && group.matcher != null ? group.matcher : null;
      if (FORCE_NO_MATCHER.has(label)) matcher = null;
      (group.hooks || []).forEach((handler, handlerIndex) => {
        if (!handler || handler.type !== 'command') return;
        const identity = { event_name: label };
        if (matcher != null) identity.matcher = matcher;
        identity.hooks = [normalizedHandler(handler)];
        const key = `${abs}:${label}:${groupIndex}:${handlerIndex}`;
        entries.push({ key, trusted_hash: versionForToml(identity) });
      });
    });
  }
  return { abs, entries };
}

function tomlQuote(s) {
  // 基础字符串引用；路径不含 " 或 \，安全。
  return '"' + String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"';
}

function main() {
  const target = process.argv[2];
  if (!target) {
    process.stderr.write('usage: node codex-trust.cjs <path to .codex/hooks.json>\n');
    process.exit(2);
  }
  const { abs, entries } = computeEntries(target);
  const lines = [];
  lines.push(`# Codex hook trust for ${abs}`);
  lines.push(`# 追加到 ~/.codex/config.toml（项目信任：仓库根已 trusted，无需另加）`);
  for (const e of entries) {
    if (e.warning) {
      lines.push(`# WARNING: ${e.warning}`);
      continue;
    }
    lines.push(`[hooks.state.${tomlQuote(e.key)}]`);
    lines.push(`trusted_hash = ${tomlQuote(e.trusted_hash)}`);
  }
  process.stdout.write(lines.join('\n') + '\n');
}

main();
