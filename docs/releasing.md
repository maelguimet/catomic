# Release Evidence and Publication

New Catomic releases are published only by the tag-triggered
[`release.yml`](../.github/workflows/release.yml) workflow. Its generated
`release-evidence.md` asset is the canonical acceptance record for one release.
It names the tag, exact source SHA, workflow run, toolchains, package, public
assets, hashes, provenance, fresh-download checks, and manual acceptance
references.

A committed roadmap acceptance document cannot contain the SHA of its own
commit without changing that SHA. Files such as
[`v0.1-acceptance.md`](v0.1-acceptance.md) therefore remain historical manual
records for the exact candidate they name. Descendant tags and the moving
`master` branch do not inherit those results. The generated release evidence is
what binds a final tag and its public bytes without a self-reference problem.
The manually assembled `v0.1.0-beta.1` prerelease predates this contract and is
documented as an explicit historical exception below.

## Maintainer Procedure

Before tagging:

1. Update `Cargo.toml` and `Cargo.lock` to the intended release version.
2. Complete the [Linux compatibility matrix](compatibility.md) with
   `build_report.py --release-candidate`. Publish the resulting JSON and
   Markdown beside the exact tested candidate binary and checksum, then link
   that durable result from the candidate acceptance record. A different
   checksum, rebuild, or expiring local path is not release evidence.
3. Require normal `master` CI to pass for the commit that will be tagged.
4. Create and push an annotated `v<package-version>` tag at that exact commit.

The release workflow then does all of the following on the tagged checkout:

- verifies that the checkout, pushed tag, event SHA, and Cargo version agree;
- records the stable and MSRV toolchains and the hosted-runner identity;
- runs formatting, Clippy, MSRV, default tests, and ignored acceptance tests;
- lists, builds, and verifies the Cargo source package;
- builds the managed release binary from that same checkout;
- emits per-binary and complete SHA-256 manifests;
- creates GitHub/Sigstore provenance for the binary, packaged source, and
  attached verification metadata;
- publishes release notes linked to the exact source, workflow, provenance,
  historical acceptance, and pending final evidence;
- starts a fresh runner, downloads the public assets, and independently checks
  checksums, architecture, version, provenance, installed bytes, and basic PTY
  startup/teardown;
- uploads `release-evidence.md` only after those public checks pass.

The standalone verifier can also check an already-downloaded asset set:

```sh
scripts/verify-release.sh \
  /path/to/assets \
  v0.1.0-beta.2 \
  0123456789abcdef0123456789abcdef01234567 \
  catomic-x86_64-unknown-linux-gnu
```

Consumers can additionally verify provenance with GitHub CLI:

```sh
gh attestation verify catomic-x86_64-unknown-linux-gnu \
  --repo maelguimet/catomic
```

Provenance proves which workflow and source produced particular bytes. It does
not prove that another machine will reproduce those bytes. Catomic does not
claim byte-for-byte reproducible builds until that has been independently
demonstrated.

## Failed or Corrected Releases

Release tags are never moved, and published assets are never overwritten or
deleted to conceal a mistake. The workflow deliberately does not use
`gh release upload --clobber`. If publication or fresh public verification
fails after a release exists, the workflow attaches a
`VERIFICATION-FAILED-<run-id>.md` marker and the run remains failed.

A correction requires a new version and tag. Keep the old release visible,
mark it as failed or superseded in its notes, and link both releases to each
other. This preserves what users could actually have downloaded and prevents a
checksum or provenance record from silently changing underneath them.
