#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';

const appId = '6828712684';
const configPath = process.argv[2] ?? path.join(os.homedir(), '.config/askhuman-release/app-store-connect.json');

async function main() {
  if (['--help', '-h'].includes(process.argv[2])) {
    console.log('Usage: node status.mjs [app-store-connect.json]\nRead-only: show recent builds, internal group membership, and tester state counts.');
    return;
  }
  const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
  const keyStat = fs.statSync(config.privateKeyPath);
  if (keyStat.uid !== process.getuid() || (keyStat.mode & 0o077) !== 0) {
    throw new Error('The configured API private key must be owner-only.');
  }
  const key = crypto.createPrivateKey(fs.readFileSync(config.privateKeyPath));
  if (key.asymmetricKeyType !== 'ec' || key.asymmetricKeyDetails.namedCurve !== 'prime256v1') {
    throw new Error('Expected an ES256 App Store Connect API key.');
  }
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const now = Math.floor(Date.now() / 1000);
  const input = encode({ alg: 'ES256', kid: config.keyId, typ: 'JWT' }) + '.'
    + encode({ iss: config.issuerId, iat: now, exp: now + 300, aud: 'appstoreconnect-v1' });
  const signature = crypto.sign('sha256', Buffer.from(input), { key, dsaEncoding: 'ieee-p1363' }).toString('base64url');
  const authorization = `Bearer ${input}.${signature}`;
  async function get(endpoint, params = {}) {
    const url = new URL('https://api.appstoreconnect.apple.com/v1/' + endpoint);
    url.search = new URLSearchParams(params).toString();
    const response = await fetch(url, { headers: { Authorization: authorization }, signal: AbortSignal.timeout(30000) });
    const body = await response.json();
    if (!response.ok) {
      const detail = (body.errors ?? []).map(error => error.detail ?? error.title).join('; ');
      throw new Error(`Apple API ${response.status}: ${detail}`);
    }
    return body;
  }
  const app = await get(`apps/${appId}`, { 'fields[apps]': 'name,bundleId' });
  const builds = await get('builds', {
    'filter[app]': appId, sort: '-uploadedDate', limit: '10', include: 'preReleaseVersion',
    'fields[builds]': 'version,processingState,uploadedDate,expired,preReleaseVersion',
    'fields[preReleaseVersions]': 'version,platform',
  });
  const versions = new Map((builds.included ?? []).map(item => [item.id, item.attributes.version]));
  const groups = await get(`apps/${appId}/betaGroups`, {
    'fields[betaGroups]': 'name,isInternalGroup,hasAccessToAllBuilds',
  });
  const internalGroups = [];
  for (const group of groups.data.filter(group => group.attributes.isInternalGroup)) {
    const assigned = await get(`betaGroups/${group.id}/builds`, { 'fields[builds]': 'version,processingState', limit: '200' });
    const testers = await get(`betaGroups/${group.id}/betaTesters`, { 'fields[betaTesters]': 'state', limit: '200' });
    const testerStates = {};
    for (const tester of testers.data) {
      const state = tester.attributes.state;
      testerStates[state] = (testerStates[state] ?? 0) + 1;
    }
    internalGroups.push({
      name: group.attributes.name, allBuilds: group.attributes.hasAccessToAllBuilds,
      buildIds: assigned.data.map(build => build.id), testerStates,
    });
  }
  console.log(JSON.stringify({
    app: { id: appId, ...app.data.attributes },
    builds: builds.data.map(build => ({
      id: build.id, marketingVersion: versions.get(build.relationships?.preReleaseVersion?.data?.id),
      ...build.attributes,
    })),
    internalGroups,
  }, null, 2));
}

main().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});
