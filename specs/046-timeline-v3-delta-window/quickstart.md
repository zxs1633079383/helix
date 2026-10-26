# Quickstart: Timeline V3 Delta Window Validation

This guide describes evidence order. Commands must run from the owning repository, preserve existing
dirty files and write bounded outputs under this feature's `evidence/` directory before R4.

## 1. Contract admission

```bash
python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance
```

Expected before implementation: `IMPLEMENTATION_ADMITTED`. R1/R2/R3 must all be current.

## 2. Helix focused producer validation

Run once after the complete Helix module batch, following the repository Cargo discipline:

```bash
cargo check -p helix-im
cargo test -p helix-im --test timeline_v3_contract_test
cargo test -p helix-im --test g10b_wdio_fixture_contract_test
cargo test -p helix-im --test g10c_wdio_fixture_contract_test
cargo test -p helix-im --test g10d_wdio_fixture_contract_test
cargo test -p helix-im --test g11a_wdio_fixture_contract_test
```

Expected:

- page sizes 20 and 60 pass; 0 and 61 fail before I/O;
- locate target index and short-side refill match the contract;
- ordinary updates contain no Replace or extra Activate;
- every declared canonical file and provenance hash matches producer output.

Then run the repository's focused fixture/Strict Matrix validators. Epoch 12 must report 50 total,
47 active and three excluded.

## 3. PC consumer validation

From `/System/Volumes/Data/workspace/angular/cses-client`:

```bash
node scripts/message-wdio/sync-helix-fixtures.mjs --check
npx jest -c tests/e2e/wdio/jest.config.js --runInBand
npm run message:wdio:contracts
node scripts/message-wdio/check-production-boundary.mjs
```

Run G-10b/G-10c/G-10d/G-11a mock lanes independently. Run single-real and dual-real only when the
native driver, fixed Helix revision and required accounts are available. Mock does not prove
persistence, WS or native rendering.

Expected operation-level outcomes:

- loadNewer extends the end without activation;
- full append keeps the visible window at `pageSize` and stable rows;
- anchored append preserves pixel offset and increments hint; click returns latest and clears it;
- uncached locate installs/activates current token and supports older/newer pagination.

## 4. Mobile consumer validation

From `/System/Volumes/Data/workspace/c/mobile-qucik-c++`, first rebuild the runtime against the
declared Helix source revision, then run the repository's focused SurfaceRuntime and Gate scripts:

```bash
scripts/build-helix-ffi.sh
scripts/run-surface-runtime-gtest.sh
scripts/run-uc01-16-gtest.sh
```

The exact script subset may be narrowed by the implemented Gate runner, but the output must name
G-10b/G-10c/G-10d/G-11a and prove the same normalized state as PC.

## 5. Parity and hard cut

Run byte-exact fixture parity first, then semantic parity over operations, keys, ranges, bounds,
watermarks, revisions and active window. Finally scan producer and consumer positive Timeline paths
for V2 `timelinePatch + replaceView`.

Expected:

- Helix, PC and Mobile fixture bytes and declared hashes match;
- normalized Timeline state matches for every canonical file;
- positive Timeline V2 generation/consumption count is zero;
- unaffected non-Timeline V2 regression assets still pass.

## 6. Record evidence

For each successful real validator:

```bash
python3 .specify/extensions/helix-loop/scripts/loop_engine.py record-evidence \
  --id V-00X \
  --command '<actual command>' \
  --started-at '<ISO-8601>' \
  --ended-at '<ISO-8601>' \
  --exit-code 0 \
  --output-file 'specs/046-timeline-v3-delta-window/evidence/<file>'
```

Do not mark a task complete until its named validator output exists. Missing real prerequisites are
`blocked`, not replaced by fixture/mock evidence.
