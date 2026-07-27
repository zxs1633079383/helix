# G-S1 Implementation Evidence

- Helix: `cargo check -p helix-im` exit 0; focused tests 16/16.
- PC contracts: Jest 35/35; WDIO TypeScript check exit 0; boundary 2735/2735.
- PC candidate: build completed after restoring ignored local resources.
- PC runtime blocker: `src/framework/setup.rs:75`,
  `app.path().resource_dir() -> UnknownPath`, then SIGABRT before WebDriver ready.
- Mobile: fixture parity exit 0; GS1 Frame 4/4; GS01 non-real 6/6;
  hard-cut and source-pin exit 0.

Evidence boundary: contract/unit/build success is not PC Mock or Real Green.
The next admissible action is to repair or supply a bundle-aware WDIO candidate,
rerun GS1 Mock/single-real/dual-real/report, then execute Mobile RealChain and R4.
