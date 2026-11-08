'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { registerHooks } = require('node:module');
const { pathToFileURL } = require('node:url');
const { Script } = require('node:vm');

const BOOT_ENV = 'ASKHUMAN_CURSOR_CLI_BOOT';
const INDEX = 'index.js';
// The same behavior checks apply across builds; fingerprints identify the exact inputs tested.
const BUILDS = {
  // 2026.10.01-e373342
  f6bd8dece34b56431ee74f1ac827e032d054381085e7da7088e85abdd1008330: {
    interactive: ['9969.index.js', '7d62ffa7d2ce4c39cd791a6f592f6bcf856f23533f51ff8e9b931e68f85816ad'],
    registry: ['2320.index.js', '14c1c9c5e3b1626c5e4e79e148f0d72ca354d49334870c90b8b9c43923e6f708'],
  },
  // 2026.10.14-0f9edde
  '335ce100d5f0aa60833e0ff773542dd7042afa0dacde61f01265cfa4f750e6ea': {
    interactive: ['9627.index.js', '54771412a04600f3011de85956d0e46bac6f97dacb7a033f1a29a9721f7c402e'],
    registry: ['4330.index.js', '0076294c3864b73812e9262cb8e54ddb39847908cdfb4cff3e893956ecf1fdcd'],
  },
  // 2026.11.05-86047df
  '932a6744cb9cd5e06b9ef292a1dc58bc17e69f8a4f72c21e7cb572fa0561d15b': {
    interactive: ['1706.index.js', '146dd450f58e927f99b6d3674d6bccd9cb0b4946883f95a145a90e377207b1b7'],
    registry: ['8283.index.js', 'bdb15dd57292cab335ff2a23f3c16fabb959a43af3bfac911307ffe789241d50'],
  },
};

function digest(source) {
  return createHash('sha256').update(source).digest('hex');
}

function replaceOne(source, original, replacement) {
  const start = source.indexOf(original);
  if (start < 0 || source.indexOf(original, start + original.length) >= 0) {
    throw new Error('expected one matching vendor code location');
  }
  return source.slice(0, start) + replacement + source.slice(start + original.length);
}

function singleMatch(source, pattern) {
  const matches = [...source.matchAll(pattern)];
  if (matches.length !== 1) throw new Error('expected one matching vendor code location');
  return matches[0];
}

function stallPolicy(options) {
  const config = options.stallDetectorConfig;
  const failTimeout = config?.failTimeoutMs;
  if (Number.isFinite(failTimeout) && failTimeout <= 0) return { options, windowMs: 0 };
  return {
    options: { ...options, stallDetectorConfig: {
      ...config, failTimeoutMs: Math.max(300000, Number.isFinite(failTimeout) ? failTimeout : 0),
    } },
    windowMs: 600000,
  };
}

function wrapStallClassifier(nativeClassify, minimumWindow) {
  return function (error, state, ...rest) {
    const elapsed = state.stallRetryElapsedMs;
    if (Number.isFinite(elapsed) && elapsed >= 0 && elapsed < minimumWindow) {
      state = { ...state, stallRetryElapsedMs: undefined };
    }
    return nativeClassify.call(this, error, state, ...rest);
  };
}

function patchStall(source) {
  const retry = singleMatch(source,
    /runOptions:([\w$]+),requestId:[\w$]+,(?:loop:this\.options\.loop\?\?"nal",)?notifyInitialResume:!0,logging:/g);
  const runStart = source.lastIndexOf('async run(', retry.index);
  const signature = /^async run\(([\w$,]+)\)\{/.exec(source.slice(runStart, retry.index));
  const options = retry[1];
  if (signature?.[1].split(',').at(-1) !== options) throw new Error('unexpected retry run options');
  const insertAt = runStart + signature[0].length;
  const classify = singleMatch(source.slice(retry.index, retry.index + 8000),
    /\},classify:([\s\S]+?),buildNonRetriedErrorInfo:/g);
  const classification = `},classify:(${wrapStallClassifier.toString()})(${classify[1]},ahStall.windowMs),buildNonRetriedErrorInfo:`;
  const rewritten = replaceOne(source, classify[0], classification);
  const snapshot = `const ahStall=(${stallPolicy.toString()})(${options});${options}=ahStall.options;`;
  return rewritten.slice(0, insertAt) + snapshot + rewritten.slice(insertAt);
}

function settleBackgroundCompletions(registry, eligible) {
  const blocked = eligible.filter(({ payload }) => payload.kind === 'shell' || payload.kind === 'subagent');
  if (!blocked.length) return eligible;
  const ids = blocked.map(({ id }) => id);
  try {
    registry.ack(ids);
  } catch {
    // Leave settlement to the native queue if ACK fails, without starting a model turn.
    registry.nack(ids, { requeue: true });
  }
  const blockedIds = new Set(ids);
  return eligible.filter(({ id }) => !blockedIds.has(id));
}

function patchWake(source) {
  const gate = singleMatch(source,
    /if\(([\w$]+\.backgroundWorkRegistry)\.nack\(([\w$]+)\.map\(\(([\w$]+)=>\3\.id\)\),\{requeue:!0\}\),0===([\w$]+)\.length\)return;(?=try\{const [\w$]+=\{stack:\[\],error:void 0,hasError:!1\};try\{const [\w$]+=\4\.map)/g);
  const [original, registry, awaiting, , eligible] = gate;
  const callSite = source.slice(gate.index + original.length, gate.index + original.length + 850);
  if (!callSite.includes('case:"backgroundTaskCompletionAction"') || !callSite.includes(`${registry}.ack(${eligible}.map`)) {
    throw new Error('unexpected background completion dispatch');
  }
  return replaceOne(source, original, `
    ${registry}.nack(${awaiting}.map(item => item.id), {requeue: true});
    const ahEligible = (${settleBackgroundCompletions.toString()})(${registry}, ${eligible});
    if (ahEligible.length === 0) return;
    for (let ahIndex = 0; ahIndex < ahEligible.length; ahIndex++) ${eligible}[ahIndex] = ahEligible[ahIndex];
    ${eligible}.length = ahEligible.length;
  `);
}

function failMaxResume(reason) {
  process.stderr.write(`AskHuman: Cursor CLI Max resume protection stopped Cursor (${reason}).\n`);
  process.exit(1);
}

function selectionUsesMax(manager, modelId, selection, config) {
  const model = manager.parameterizedModelMap.get(modelId);
  const variant = model?.variants.find(variant => manager.sameParameterValues(variant.parameterValues, selection.parameters));
  return selection.requiresMaxMode === true || model?.supportsNonMaxMode === false || variant?.isMaxMode === true
    || (selection.parameters.length > 0 && !variant)
    || manager.createModelDetailsFromSelection(modelId, selection.parameters, config)?.maxMode === true;
}

function currentModelIsNonMax(manager, config) {
  const current = manager.buildRequestedModel();
  return current?.maxMode === false && !ahSelectionUsesMax(manager, current.modelId, current, config);
}

function chooseNonMaxResume(manager, modelId, selection, config) {
  if (!ahSelectionUsesMax(manager, modelId, selection, config)) return selection;

  // Cursor's own picker treats unchanged parameters with Max off as a request for a non-Max variant.
  const parameters = selection.parameters;
  const alternative = manager.sanitizeParametersForModelChange(modelId, parameters, {
    previousParameters: parameters, maxMode: false,
  });
  if (!ahSelectionUsesMax(manager, modelId, alternative, config)) {
    process.stderr.write(`AskHuman: Resuming with a non-Max variant of ${modelId}.\n`);
    return alternative;
  }
  if (!ahCurrentModelIsNonMax(manager, config)) ahFailMaxResume('no current non-Max model is available');
  process.stderr.write(`AskHuman: Cannot restore a non-Max variant of ${modelId}. Keeping ${manager.getCurrentModel().modelId}.\n`);
  return null;
}

function patchMaxResume(source) {
  if (!source.includes('"./src/models/index.ts"') || !source.includes('this.sameParameterValues(')
      || !source.includes('requiresMaxMode:!1') || !source.includes('maxModeAutoEnabled')
      || !/createModelDetailsFromSelection\([\w$,]+\)\{/.test(source)
      || !source.includes('buildRequestedModel(){') || !source.includes('hasFixedNonMaxModelCatalog(){')) {
    throw new Error('missing Cursor model selection interfaces');
  }
  const restore = singleMatch(source,
    /setModelFromStoredId\(([\w$]+),([\w$]+)\)\{[\s\S]+?(?=mapModelToParameterizedSelection\([\w$,]+\)\{)/g);
  const [originalRestore, , restoreConfig] = restore;
  const calls = [...originalRestore.matchAll(/\b(await|yield) this\.setCurrentModelWithParameters\(([\w$.]+),([\w$]+|\[\]),([\w$]+)\),!0/g)];
  if (!calls.length || calls.length !== originalRestore.split('this.setCurrentModelWithParameters(').length - 1
      || calls.some(call => call[4] !== restoreConfig) || originalRestore.includes('.transform(')) {
    throw new Error('unexpected restored model dispatch');
  }
  let guardedRestore = originalRestore;
  for (const [call, wait, model, params, config] of calls) {
    guardedRestore = replaceOne(guardedRestore, call,
      `!1!==(${wait} this.setCurrentModelWithParameters(${model},${params},${config},!0))`);
  }
  const sameModel = singleMatch(guardedRestore,
    /\)return\(0,[\w$.]+\)\(`Stored model "\$\{[\w$]+\}" already selected; skipping catalog wait`\),!0;/g);
  guardedRestore = replaceOne(guardedRestore, sameModel[0],
    `&&(!0===${restoreConfig}.get().maxMode||this.hasFixedNonMaxModelCatalog()||ahCurrentModelIsNonMax(this,${restoreConfig}))${sameModel[0]}`);

  const setter = singleMatch(source,
    /setCurrentModelWithParameters\([\w$]+,[\w$]+,([\w$]+)\)\{[\s\S]+?(?=resolveMaxModePersistence\([\w$,]+\)\{)/g);
  const [originalSetter, config] = setter;
  const selection = singleMatch(originalSetter,
    /const ([\w$]+)=this\.sanitizeParametersForModelChange\(([\w$]+),[\w$]+,\{previousParameters:[^{}]+?,maxMode:([\w$]+)\.get\(\)\.maxMode\}\),/g);
  if (selection[3] !== config || !originalSetter.includes(`${config}.transform(`)
      || !originalSetter.includes(`this.resolveMaxModePersistence(${selection[1]}.requiresMaxMode,${config})`)) {
    throw new Error('unexpected Max persistence path');
  }
  const [select, state, modelId] = selection;
  const guardedSetter = replaceOne(originalSetter, select, `let ${select.slice(6, -1)};
    if(ahRestoringModel&&${config}.get().maxMode!==true&&!this.hasFixedNonMaxModelCatalog()){
      ${state}=ahChooseNonMaxResume(this,${modelId},${state},${config});
      if(${state}===null)return false;
    }
    const `).replace(/^setCurrentModelWithParameters\(([^)]+)\)/, 'setCurrentModelWithParameters($1,ahRestoringModel)');
  const patched = replaceOne(replaceOne(source, originalRestore, guardedRestore), originalSetter, guardedSetter);
  // Keep any vendor directive, including strict mode, at the head of the module.
  const helpers = `\nconst ahFailMaxResume=${failMaxResume};\nconst ahSelectionUsesMax=${selectionUsesMax};\n` +
    `const ahCurrentModelIsNonMax=${currentModelIsNonMax};\nconst ahChooseNonMaxResume=${chooseNonMaxResume};\n`;
  return patched + helpers;
}

function readSource(packageDir, file) {
  const filename = path.join(packageDir, file);
  const info = fs.lstatSync(filename);
  if (!info.isFile() || info.size > 16 * 1024 * 1024) throw new Error(`unexpected file for ${file}`);
  return fs.readFileSync(filename);
}

function verifiedSource(packageDir, [file, hash]) {
  const source = readSource(packageDir, file);
  if (digest(source) !== hash) throw new Error(`unverified ${file}`);
  return source;
}

function warn(feature, error) {
  process.stderr.write(`AskHuman: Cursor CLI ${feature} protection unavailable (${error.message}).\n`);
}

function hasMaxResumeRoute(source, moduleName) {
  const marker = `"./src/${moduleName}"(`;
  const start = source.indexOf(marker);
  if (start < 0) return false;
  const end = source.indexOf('},"./src/', start + marker.length);
  const module = source.slice(start, end < 0 ? undefined : end);
  return /\.setModelFromStoredId\([\w$.]+\.getMetadata\("lastUsedModel"\),[\w$.]+\.configProvider\)/.test(module)
    || /\b([\w$]+)=[\w$.]+\.getMetadata\("lastUsedModel"\)[^{}]{0,180}\.setModelFromStoredId\(\1,[\w$.]+\.configProvider\)/.test(module);
}

function findMaxResumePatch(packageDir) {
  const models = [];
  const startup = [];
  const interactive = [];
  const files = fs.readdirSync(packageDir, { withFileTypes: true })
    .filter(file => file.isFile() && /^(?:\d+\.)?index\.js$/.test(file.name));
  if (files.length > 256) throw new Error('unexpected Cursor CLI bundle layout');
  for (const file of files) {
    const source = readSource(packageDir, file.name);
    const body = source.toString('utf8');
    const isModelManager = body.includes('"./src/models/index.ts"(') && body.includes('setModelFromStoredId(');
    const isStartup = hasMaxResumeRoute(body, 'run-agent.tsx');
    const isInteractive = hasMaxResumeRoute(body, 'ui.tsx');
    if (!isModelManager && !isStartup && !isInteractive) continue;
    const asset = [file.name, digest(source)];
    if (isModelManager) {
      models.push({ asset, body });
    }
    if (isStartup) startup.push(asset);
    if (isInteractive) interactive.push(asset);
  }
  if (models.length !== 1 || startup.length !== 1 || interactive.length !== 1) {
    throw new Error('cannot identify the Cursor resume interfaces');
  }
  const [{ asset, body }] = models;
  const transformed = patchMaxResume(body);
  new Script(transformed, { filename: asset[0] });
  return { asset, routes: [...new Set([...startup, ...interactive])], transformed };
}

function install(packageDir) {
  const maxResume = findMaxResumePatch(packageDir);
  const mainHash = digest(readSource(packageDir, INDEX));
  const build = BUILDS[mainHash];
  const patches = new Map();
  if (!build) warn('stall timeout and background wake-up', new Error('unverified Cursor CLI build'));
  for (const [asset, feature, transform, dependencies] of build ? [
    [[INDEX, mainHash], 'stall timeout', patchStall, []],
    [build.interactive, 'background wake-up', patchWake, [build.registry]],
  ] : []) {
    try {
      for (const dependency of dependencies) verifiedSource(packageDir, dependency);
      const source = verifiedSource(packageDir, asset);
      patches.set(pathToFileURL(path.join(packageDir, asset[0])).href, {
        asset, feature, dependencies, transformed: transform(source.toString('utf8')),
      });
    } catch (error) {
      warn(feature, error);
    }
  }
  const maxUrl = pathToFileURL(path.join(packageDir, maxResume.asset[0])).href;
  const otherPatch = patches.get(maxUrl);
  if (otherPatch) maxResume.transformed = patchMaxResume(otherPatch.transformed);
  patches.set(maxUrl, { ...maxResume, dependencies: otherPatch?.dependencies ?? [] });
  const routeChecks = new Map(maxResume.routes.map(asset => [pathToFileURL(path.join(packageDir, asset[0])).href, asset]));
  const hooks = registerHooks({
    load(url, context, nextLoad) {
      let result = nextLoad(url, context);
      const route = routeChecks.get(url);
      if (route) {
        try {
          const source = checkLoadedSource(packageDir, route, result);
          result = { ...result, format: 'commonjs', source };
        } catch (error) {
          failMaxResume(error.message);
        }
        routeChecks.delete(url);
      }
      const patch = patches.get(url);
      patches.delete(url);
      if (!patches.size && !routeChecks.size) hooks.deregister();
      if (!patch) return result;
      try {
        for (const dependency of patch.dependencies) verifiedSource(packageDir, dependency);
        checkLoadedSource(packageDir, patch.asset, result);
        return { ...result, format: 'commonjs', source: patch.transformed };
      } catch (error) {
        if (url === maxUrl) failMaxResume(error.message);
        warn(patch.feature, error);
        return result;
      }
    },
  });
}

function checkLoadedSource(packageDir, asset, result) {
  const source = result.source == null ? verifiedSource(packageDir, asset)
    : typeof result.source === 'string' ? result.source : Buffer.from(result.source);
  // The vendor package can leave the CommonJS format unspecified.
  if ((result.format != null && result.format !== 'commonjs') || digest(source) !== asset[1]) {
    throw new Error(`unexpected source for ${asset[0]}`);
  }
  return source;
}

const boot = process.env[BOOT_ENV];
if (boot) {
  delete process.env[BOOT_ENV];
  try {
    const { package: packageDir, originalOptions } = JSON.parse(boot);
    if (originalOptions === null) delete process.env.NODE_OPTIONS;
    else process.env.NODE_OPTIONS = originalOptions;
    // The vendor launcher probes `node --version` before executing its main script.
    if (process.argv[1]) {
      if (fs.realpathSync(process.argv[1]) !== path.join(packageDir, INDEX)
          || fs.realpathSync(process.execPath) !== path.join(packageDir, 'node')) {
        throw new Error('unexpected Cursor CLI entry point');
      }
      install(packageDir);
    }
  } catch (error) {
    failMaxResume(error.message);
  }
}

module.exports = { BUILDS, findMaxResumePatch, patchStall, patchWake, settleBackgroundCompletions, stallPolicy, wrapStallClassifier };
