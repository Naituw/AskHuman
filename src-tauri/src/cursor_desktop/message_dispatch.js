else if (request.op === "message") {
  await refresh();
  const matches = [];
  for (const record of windows.values()) {
    if (record.phase !== "active" || record.wc.isDestroyed()) continue;
    const owns = await evaluate(record.wc, `globalThis[${JSON.stringify(rendererKey)}]?.owns(${JSON.stringify(request.operation.sessionId)}) === true`);
    if (owns) matches.push(record.wc);
  }
  if (matches.length !== 1) result = { status: "rejected", reason: matches.length ? "nativeUnavailable" : "notFound" };
  else result = await evaluate(matches[0], `globalThis[${JSON.stringify(rendererKey)}].send(${JSON.stringify(request.operation)})`);
}
