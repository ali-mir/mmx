// workload.js
// Randomized CRUD workload for mongosh (parallel async workers).
//
// Usage:
//   mongosh "<uri>/<db>" workload.js -- --coll=test --workers=8 --seconds=60
//
// Options (all optional):
//   --coll=<name>           Collection name (default: "workload")
//   --workers=<n>           Concurrent workers (default: 4)
//   --seconds=<n>           Duration (default: 30)
//   --opsPerWorker=<n>      Stop after N ops per worker (0 = unlimited, default: 0)
//   --pInsert=<0..1>        Probability insert (default: 0.25)
//   --pFind=<0..1>          Probability find (default: 0.35)
//   --pUpdate=<0..1>        Probability update (default: 0.30)
//   --pDelete=<0..1>        Probability delete (default: 0.10)
//   --docSize=<bytes>       Approx payload size for inserts (default: 256)
//   --useTxn=<true|false>   Wrap each op in a transaction (default: false)
//   --seed=<int>            RNG seed (default: 12345)
//   --createIndexes=<true|false> (default: true)

"use strict";


  // -----------------------
  // Progress bar rendering
  // -----------------------

function monoMs() {
  // Monotonic-ish clock when available; otherwise fall back to wall clock.
  if (
    typeof process !== "undefined" &&
    process.hrtime &&
    typeof process.hrtime.bigint === "function"
  ) {
    return Number(process.hrtime.bigint() / 1_000_000n);
  }
  return Date.now();
}

function fmt(n) {
  return n.toLocaleString("en-US");
}

function renderBar(pct, width) {
  const clamped = Math.max(0, Math.min(1, pct));
  const filled = Math.round(clamped * width);
  const empty = width - filled;
  return `[${"#".repeat(filled)}${"-".repeat(empty)}]`;
}

function writeLine(s) {
  // rewrite the current line
  if (typeof process !== "undefined" && process.stdout && process.stdout.write) {
    process.stdout.write(`\r${s}`);
  } else {
    print(s);
  }
}

  // -----------------------
  // Arg parsing
  // -----------------------
  function parseArgs(argv) {
    const out = {};
    for (let i = 0; i < argv.length; i++) {
      const a = argv[i];
      if (!a.startsWith("--")) continue;
      const eq = a.indexOf("=");
      if (eq !== -1) {
        out[a.slice(2, eq)] = a.slice(eq + 1);
      } else {
        const k = a.slice(2);
        const v = (i + 1 < argv.length && !argv[i + 1].startsWith("--")) ? argv[++i] : "true";
        out[k] = v;
      }
    }
    return out;
  }

  function toInt(v, def) {
    if (v === undefined) return def;
    const n = parseInt(v, 10);
    return Number.isFinite(n) ? n : def;
  }
  function toNum(v, def) {
    if (v === undefined) return def;
    const n = Number(v);
    return Number.isFinite(n) ? n : def;
  }
  function toBool(v, def) {
    if (v === undefined) return def;
    if (typeof v === "boolean") return v;
    return ["1", "true", "yes", "y", "on"].includes(String(v).toLowerCase());
  }

  const args = globalThis.__args || {};

  const collName = args.coll || "workload";
  const workers = Math.max(1, toInt(args.workers, 4));
  const seconds = Math.max(1, toInt(args.seconds, 30));
  const opsPerWorker = Math.max(0, toInt(args.opsPerWorker, 0));

  const pInsert = toNum(args.pInsert, 0.25);
  const pFind = toNum(args.pFind, 0.35);
  const pUpdate = toNum(args.pUpdate, 0.30);
  const pDelete = toNum(args.pDelete, 0.10);
  const docSize = Math.max(32, toInt(args.docSize, 256));

  const useTxn = toBool(args.useTxn, false);
  const seed = toInt(args.seed, 12345);
  const createIndexes = toBool(args.createIndexes, true);

  const pSum = pInsert + pFind + pUpdate + pDelete;
  if (Math.abs(pSum - 1.0) > 1e-6) {
    print(`WARN: probabilities sum to ${pSum.toFixed(3)} (expected 1.0). Normalizing.`);
  }

  // -----------------------
  // Deterministic RNG (LCG)
  // -----------------------
  function makeRng(seed0) {
    let s = (seed0 >>> 0) || 1;
    return {
      nextU32() {
        // Numerical Recipes LCG
        s = (Math.imul(1664525, s) + 1013904223) >>> 0;
        return s;
      },
      nextFloat() {
        // [0, 1)
        return this.nextU32() / 0x100000000;
      },
      nextInt(max) {
        return (this.nextU32() % max) | 0;
      }
    };
  }

  // -----------------------
  // Workload helpers
  // -----------------------
  function nowMs() {
    return (typeof Date !== "undefined") ? Date.now() : new Date().getTime();
  }

  function randomPayload(rng, approxBytes) {
    // Build a string payload ~ approxBytes (not exact, but close).
    // Use base64-ish chars for compactness.
    const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const len = Math.max(8, approxBytes);
    let s = "";
    // chunk to avoid quadratic concatenations
    const chunk = 1024;
    for (let i = 0; i < len; i += chunk) {
      const n = Math.min(chunk, len - i);
      let part = "";
      for (let j = 0; j < n; j++) {
        part += chars[rng.nextInt(chars.length)];
      }
      s += part;
    }
    return s;
  }

  function pickOp(rng) {
    // Normalize probabilities if they don't sum to 1
    const sum = pSum || 1;
    const r = rng.nextFloat() * sum;
    if (r < pInsert) return "insert";
    if (r < pInsert + pFind) return "find";
    if (r < pInsert + pFind + pUpdate) return "update";
    return "delete";
  }

  function keySpaceForWorker(workerId) {
    // Separate keyspaces per worker to reduce hot contention if desired.
    // Keep it stable to allow operations to hit existing docs.
    return { base: workerId * 10_000_000, span: 5_000_000 };
  }

  function randomKey(rng, ks) {
    return ks.base + rng.nextInt(ks.span);
  }

  // -----------------------
  // Mongo setup
  // -----------------------
  const dbName = db.getName();
  const coll = db.getCollection(collName);

  print(`DB: ${dbName}, Coll: ${collName}`);
  print(`Workers: ${workers}, Duration: ${seconds}s, opsPerWorker: ${opsPerWorker || "unlimited"}`);
  print(`Ops mix: insert=${pInsert}, find=${pFind}, update=${pUpdate}, delete=${pDelete} (sum=${pSum})`);
  print(`docSize≈${docSize}B, useTxn=${useTxn}, seed=${seed}`);

  if (createIndexes) {
    // A couple useful indexes for common query paths.
    // `k` is our primary synthetic key; `ts` can support time-ish queries.
    coll.createIndex({ k: 1 }, { unique: false });
    coll.createIndex({ ts: -1 });
  }

  // -----------------------
  // Per-worker loop
  // -----------------------
  function newStats() {
    return {
      insert: 0, find: 0, update: 0, delete: 0,
      ok: 0, err: 0,
      latMsSum: 0,
      lastErr: null
    };
  }

  async function withOptionalTxn(fn) {
    if (!useTxn) return await fn(null);

    // Transactions require replica set or sharded cluster with appropriate config.
    const session = db.getMongo().startSession();
    try {
      const sdb = session.getDatabase(dbName);
      const scoll = sdb.getCollection(collName);
      let res;
      await session.withTransaction(async () => {
        res = await fn({ session, scoll });
      });
      return res;
    } finally {
      session.endSession();
    }
  }

  async function workerLoop(workerId, deadlineMs) {
    const rng = makeRng((seed + workerId * 1337) | 0);
    const ks = keySpaceForWorker(workerId);
    const stats = newStats();
    let ops = 0;

    while (nowMs() < deadlineMs && (opsPerWorker === 0 || ops < opsPerWorker)) {
      ops++;
      const op = pickOp(rng);
      const k = randomKey(rng, ks);

      const t0 = nowMs();
      try {
        await withOptionalTxn(async (txn) => {
          const c = txn ? txn.scoll : coll;
          const sessionOpt = txn ? { session: txn.session } : undefined;

          if (op === "insert") {
            const doc = {
              k,
              wid: workerId,
              ts: new Date(),
              payload: randomPayload(rng, docSize),
              n: rng.nextInt(1_000_000),
            };
            await c.insertOne(doc, sessionOpt);
            stats.insert++;
          } else if (op === "find") {
            // Mix of point lookup and small range.
            if (rng.nextFloat() < 0.7) {
              await c.findOne({ k }, sessionOpt);
            } else {
              const k2 = k + rng.nextInt(200);
              await c.find({ k: { $gte: k, $lte: k2 } }, sessionOpt).limit(20).toArray();
            }
            stats.find++;
          } else if (op === "update") {
            // Upsert-ish update to keep dataset from evaporating.
            const upd = {
              $set: { ts: new Date(), n: rng.nextInt(1_000_000) },
              $inc: { hits: 1 }
            };
            await c.updateOne({ k }, upd, { ...(sessionOpt || {}), upsert: true });
            stats.update++;
          } else if (op === "delete") {
            // Delete sometimes by key, sometimes older docs.
            if (rng.nextFloat() < 0.8) {
              await c.deleteOne({ k }, sessionOpt);
            } else {
              const cutoff = new Date(Date.now() - 60_000); // 1 min
              await c.deleteMany({ ts: { $lt: cutoff }, wid: workerId }, { ...(sessionOpt || {}), limit: 50 });
            }
            stats.delete++;
          } else {
            throw new Error(`unknown op ${op}`);
          }
        });

        const dt = nowMs() - t0;
        stats.latMsSum += dt;
        stats.ok++;
      } catch (e) {
        stats.err++;
        stats.lastErr = String(e && e.message ? e.message : e);
      }

      // Small jitter to avoid lockstep workers; tune as needed.
      if (rng.nextFloat() < 0.02) {
        await new Promise((r) => setTimeout(r, 5 + rng.nextInt(20)));
      }
    }

    return stats;
  }

  function mergeStats(all) {
    const out = newStats();
    for (const s of all) {
      out.insert += s.insert;
      out.find += s.find;
      out.update += s.update;
      out.delete += s.delete;
      out.ok += s.ok;
      out.err += s.err;
      out.latMsSum += s.latMsSum;
      if (s.lastErr) out.lastErr = s.lastErr;
    }
    return out;
  }

  // -----------------------
  // Run
  // -----------------------
async function main() {
  const startMono = monoMs();
  const deadlineMono = startMono + seconds * 1000;

  // Shared progress
  const progress = {
    ops: 0,
    ok: 0,
    err: 0,
    lastErr: null,
  };

  // Wrap workerLoop so we can increment progress as we go.
  async function workerLoopWithProgress(workerId, deadlineMonoMs) {
    const rng = makeRng((seed + workerId * 1337) | 0);
    const ks = keySpaceForWorker(workerId);
    const stats = newStats();
    let ops = 0;

    while (monoMs() < deadlineMonoMs && (opsPerWorker === 0 || ops < opsPerWorker)) {
      ops++;
      progress.ops++;

      const op = pickOp(rng);
      const k = randomKey(rng, ks);

      const t0 = monoMs();
      try {
        await withOptionalTxn(async (txn) => {
          const c = txn ? txn.scoll : coll;
          const sessionOpt = txn ? { session: txn.session } : undefined;

          if (op === "insert") {
            const doc = {
              k,
              wid: workerId,
              ts: new Date(),
              payload: randomPayload(rng, docSize),
              n: rng.nextInt(1_000_000),
            };
            await c.insertOne(doc, sessionOpt);
            stats.insert++;
          } else if (op === "find") {
            if (rng.nextFloat() < 0.7) {
              await c.findOne({ k }, sessionOpt);
            } else {
              const k2 = k + rng.nextInt(200);
              await c.find({ k: { $gte: k, $lte: k2 } }, sessionOpt).limit(20).toArray();
            }
            stats.find++;
          } else if (op === "update") {
            const upd = {
              $set: { ts: new Date(), n: rng.nextInt(1_000_000) },
              $inc: { hits: 1 }
            };
            await c.updateOne({ k }, upd, { ...(sessionOpt || {}), upsert: true });
            stats.update++;
          } else if (op === "delete") {
            if (rng.nextFloat() < 0.8) {
              await c.deleteOne({ k }, sessionOpt);
            } else {
              const cutoff = new Date(Date.now() - 60_000);
              await c.deleteMany({ ts: { $lt: cutoff }, wid: workerId }, { ...(sessionOpt || {}), limit: 50 });
            }
            stats.delete++;
          } else {
            throw new Error(`unknown op ${op}`);
          }
        });

        const dt = monoMs() - t0;
        stats.latMsSum += dt;
        stats.ok++;
        progress.ok++;
      } catch (e) {
        stats.err++;
        progress.err++;
        progress.lastErr = String(e && e.message ? e.message : e);
        stats.lastErr = progress.lastErr;
      }

      if (rng.nextFloat() < 0.02) {
        await new Promise((r) => setTimeout(r, 5 + rng.nextInt(20)));
      }
    }

    return stats;
  }

  const barWidth = 28;

  const ticker = (typeof setInterval === "function") ? setInterval(() => {
    const now = monoMs();
    const elapsed = (now - startMono) / 1000;
    const total = seconds;
    const pct = Math.min(1, elapsed / total);

    const bar = renderBar(pct, barWidth);
    const opsPerSec = progress.ops / Math.max(0.001, elapsed);

    const msg =
      `${bar} ${(pct * 100).toFixed(0)}% ` +
      `${elapsed.toFixed(1)}s/${total}s ` +
      `ops=${fmt(progress.ops)} (${opsPerSec.toFixed(0)}/s) ` +
      `ok=${fmt(progress.ok)} err=${fmt(progress.err)}`;

    writeLine(msg);
  }, 1000) : null;

  const promises = [];
  for (let i = 0; i < workers; i++) promises.push(workerLoopWithProgress(i, deadlineMono));
  const results = await Promise.all(promises);

  if (ticker) clearInterval(ticker);
  writeLine(""); // clear the line
  print("");     // newline

  const endMono = monoMs();
  const durSec = Math.max(0.001, (endMono - startMono) / 1000);

  const totalStats = mergeStats(results);
  const avgLat = totalStats.ok ? (totalStats.latMsSum / totalStats.ok) : 0;
  const opsTotal = totalStats.ok + totalStats.err;

  print("\n=== Summary ===");
  printjson({
    duration_s: durSec,
    workers,
    ok: totalStats.ok,
    err: totalStats.err,
    ops_total: opsTotal,
    ops_per_s: opsTotal / durSec,
    avg_latency_ms: avgLat,
    mix: {
      insert: totalStats.insert,
      find: totalStats.find,
      update: totalStats.update,
      delete: totalStats.delete
    },
    last_error: totalStats.lastErr,
  });

}

(async () => {
  await main();
})().catch((e) => {
  print(`FATAL: ${e && e.stack ? e.stack : e}`);
  throw e;
});
