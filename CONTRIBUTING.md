# Contributing to deer-gui

Thanks for your interest in the project. This document covers the rules that this repository actually enforces, the design constraints that are easy to break by accident, and the commands worth running before you open a pull request.

Rules for AI agents working in this repository are summarized in [`agent.md`](agent.md); this file is the human-facing counterpart and holds the same discipline.

## Getting set up

- **Rust 1.85+** (the workspace uses edition 2024).
- **Windows** if you want to work on the windowing layer or anything that presents to a window. Everything else builds without it.
- **No Vulkan SDK.** `deer-vk` loads `vulkan-1.dll` at runtime, so no import library is needed.

```sh
git clone https://github.com/DeerLuuu/deer-gui.git
cd deer-gui
cargo test --workspace
```

The workspace is:

```
crates/deer-layout   language-independent core (node tree, layout, hit testing, .dui parsing)
crates/deer-gpu      GPU HAL + CPU reference backend
crates/deer-vk       Vulkan backend
crates/deer-window   windowing layer (the only third-party dependency: winit)
crates/deer-gui      facade crate + examples
test_project/        standalone example project, deliberately outside the workspace
```

`test_project/deer-hello` is excluded from the workspace on purpose (it depends back on the library by path). Verify it still builds with `cd test_project/deer-hello && cargo build` — note that its `Cargo.toml` currently pins a machine-specific path, so you may need to adjust it locally. Do not commit such a path change unless the maintainer asks for it.

## The feature completion rule

**A feature is not complete until it ships with all four of these**, in the same change:

1. **An example** at `crates/deer-gui/examples/<name>.rs`:
   - a header comment saying what it is, how to run it, and where the output goes;
   - a **self-check assertion** at the end (for example "the image must not be a single flat color"), so that "it ran successfully but did nothing" cannot pass;
   - actually run it and confirm `exit=0`.
2. **A guide** at `docs/features/<name>.md`, following [`docs/features/TEMPLATE.md`](docs/features/TEMPLATE.md).
3. **An entry in `FEATURES.md`** — status, guide link, and example command must all be correct.
4. **An update to `docs/TUTORIAL.md`** if the feature belongs to the beginner main line.

This is not a convention that relies on good intentions: `crates/deer-gui/tests/docs_consistency.rs` turns the following mistakes into **test failures**:

| Rule enforced by test | Fails when |
|---|---|
| Every `.md` link in `FEATURES.md` resolves | you link a guide that does not exist |
| Every `--example <name>` in `FEATURES.md` has `examples/<name>.rs` | you name an example that does not exist |
| Every ✅ row has **both** a guide link and an example command | you register a feature without delivering it |
| Every guide contains a "what it cannot do" section and no unchecked `- [ ]` items | you copy the template and forget to finish it |
| `README.md` links `docs/TUTORIAL.md`, `FEATURES.md`, and `docs/features/` | you drop a navigation entry while editing the README |

The reason for this strictness is recorded in `FEATURES.md`: after the M1 delivery, the library could render — but nobody knew how, because a piece of the render chain was missing and there was no user-facing documentation.

## Verification

Run the smallest check that covers your change, and read the numbers from the output rather than from any document (counts drift as milestones land).

```sh
cargo test --workspace                 # all assertions
cargo test -p deer-vk -- --nocapture   # which GPUs are enumerated on this machine
cargo run -p deer-gui --example render_to_png    # offscreen render, CI-friendly
cargo run -p deer-gui --features window --example window_preview   # real window (Windows)
```

### Gate environment variables

Some tests are **skipped by default**. A skip counts as a pass, so a green run without these variables is not evidence:

| Variable | Effect | If unset |
|---|---|---|
| `DEER_VK_WINDOW_TESTS=1` | enables the real-window end-to-end tests | they are explicitly skipped — not evidence |
| `DEER_VK_VALIDATION=1` | requests the Vulkan validation layer | the "zero validation messages" assertions lose their discriminating power |
| `DEER_HAL_FRAMES` | frame count for `hal_window_path` (default 30) | default is fine |
| `DEER_WINDOW_FRAMES`, `DEER_WINDOW_HOLD=1` | example frame count / keep the window open | human observation only |
| `DEER_FONT_DEBUG` | verbose font parsing output | debugging only |

The full M2b gate:

```powershell
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk
```

### Claims that are not evidence

Four kinds of "green" do not support a success claim:

1. **A skip** — the gate variable was unset. The repository's own wording: a skip counts as a pass, so do not treat it as evidence.
2. **Zero validation messages** — the validation layer does not do general synchronization validation, so this does not prove memory-domain dependencies are correct.
3. **A 1 LSB difference** — it is a measured ceiling, not a proven upper bound.
4. **Passing only on the windowless path** — the real-window and present paths may not be covered at all.

### How to write test counts (repository-wide convention)

Test and assertion counts change with every milestone, so **live documentation must not embed them**. When you write about a verification result:

| Where | Rule |
|---|---|
| **Live documentation** — `README.md`, `README.zh-CN.md`, `FEATURES.md`, `ROADMAP.md`, `docs/features/*.md`, `CONTRIBUTING.md`, `agent.md` | Write **completion, not quantity**: `all passed / 0 failed`, or "as many as the run printed". Do **not** write `167 passed`, `24 assertions`, `共 N 条`, or any other absolute count. |
| **Dated snapshots** — `docs/M1-report.md`, `docs/superpowers/plans/*.md` | A count is allowed **only if** it is clearly dated and marked as "measured at that time", e.g. "cargo test --workspace at the M1 tag printed the numbers below" and "as the run printed". |
| **Why** | An undated count in live docs is stale the moment the next test lands. `167 passed` was true at one commit and was already wrong one milestone later — that is the failure this rule prevents. |

If you want a number to be verifiable, do not freeze it in prose: assert it in a test, or read it from the run output. When you must reference a magnitude in live docs, prefer a **stable structural fact** instead of a count — for example "one test per layout invariant in `crates/deer-layout/tests/layout_invariants.rs`" rather than "18 tests" or "24 assertions".

### Never edit repository files through shell text pipelines

Do **not** rewrite repository files with a shell text pipeline — `Get-Content -Raw … | … | Set-Content`, in-place `-replace`, or `>>` redirection into a source or documentation file. On this machine, Windows PowerShell 5.1 reads UTF-8-without-BOM as ANSI, so **CJK text is silently rewritten as `?` (or U+FFFD), and the damage is irreversible**.

- Use the **version-guarded editor tools** (`read` / `edit` / `write`) for every file change.
- If you genuinely need a script, **write to a new file — never overwrite in place** — and specify the encoding explicitly.
- **Three incidents, all one class of bug**: `crates/deer-vk/src/windowed.rs` (M3a), `crates/deer-vk/src/device.rs` (M3b), and four feature guides (`docs/features/{gpu-geometry,gpu-offscreen,vulkan,window}.md`, C2 — 455 characters became `?`). The last one was caught by reading the file back with the `read` tool (it returned mojibake), then recovered with `git checkout --` and redone with `edit`.
- **Why this is a rule and not advice**: the damage is invisible in `git diff --stat` (the file still parses and is roughly the right size) and cannot be undone — there is no way to turn a `?` back into the character it replaced.

### After a scripted bulk rewrite, verify the result's *shape*

A scripted in-place rewrite is **not** "run it and done". Bulk edits fail by **duplicating or truncating** content far more often than by being wrong in one spot, and the file still compiles, so nothing complains.

⇒ After any scripted bulk edit, print and paste a **shape check**:

- **line count** (before vs after — must match the intended delta),
- **count of functions / tests / key symbols** (e.g. `git diff --numstat`, or `Select-String -Pattern '^fn ' | Measure-Object`),
- the specific symbols or test names you meant to add, confirmed present **exactly once**.

**Case.** A scripted rewrite of the test files wrote the content **twice** (≈1000 lines → **2956**). It was caught only by "line count + key function counts"; recovery was `git checkout --` back to HEAD, then doing the whole addition in **one** one-shot script and re-checking the shape.

This complements the previous rule: *that* one prevents **encoding** damage (`?`), *this* one prevents **structural** damage (duplicated / truncated content). Use both whenever you touch repository files with a script.

### Referring to examples in guides (docs contract)

`crates/deer-gui/tests/docs_consistency.rs` extracts every `--example <name>` from **`docs/features/*.md`** and requires `crates/deer-gui/examples/<name>.rs` to exist — **code blocks included**.

⇒ Examples that live in **another crate** (e.g. `crates/deer-vk/examples/viewport_dynamic_probe.rs`) must be referenced in guides by **source path only**. Do not write the `--example` flag together with that name inside a guide. Put the full runnable command in a non-guide document (`docs/tour/*.md`, `docs/superpowers/plans/*.md`, `ROADMAP.md`), or use a placeholder that the checker ignores:

```sh
cargo run -p deer-vk --example <file name without .rs> -- --group=0
```

### Assert the precondition, or the guard silently disables itself

When you write a guard (including a mutation case), **first assert that the code actually reached the branch you are guarding**. A false precondition **does not error** — the guard just stops testing anything.

State the precondition explicitly, right next to the assertion it protects:

- "the first frame performs **exactly one** upload" (rather than "an upload happened at some point");
- "the buffer handle **actually changed** here, and the bytes are **identical**" (rather than "a new buffer exists");
- "this frame **does**/does **not** emit a barrier" before asserting the count delta.

**This has already bitten us once**: a case built to exercise buffer replacement via *capacity growth* passed even after mutation, because an **intermediate render overwrote the recorded state** — the third step never reached the target branch. The mutation stayed green, and the guard was worthless until the precondition was asserted.

Corollary: for two-way mutations ("never emit" ⇒ red, "emit when it should not" ⇒ red), assert **both** the reaching condition and the count delta — one-sided guards hide exactly this class of failure.

### Dump the real artifact before writing a bytecode- or encoding-level criterion

When you guard a *generated* artifact (SPIR-V, bytecode, an encoded format), **print the real thing first** and write the matcher against what is actually there. Guessing from index memory produces criteria that match **something else** — and a criteria that matches something else **stays green**, so nothing tells you it is wrong.

**Case (four versions, the first three refuted by measurement).** To pin down "the unified pipeline's discriminant is `uv.x < 0`": v1 — "some comparison with one side a component-0 extract of some `vec`" — was a **no-op**: it matched the *shape* criterion (`dl < 0`) instead, so it stayed green even after the discriminant was removed. v2/v3 matched either the sampling-coordinate `Id` or the `OpLoad` result of `uv`, **missing one `OpCompositeExtract` layer** (the comparison actually consumes `OpCompositeExtract(uv, 0)`). Dumping the real bytecode settled it: `var20=uv` / `29=OpLoad(var20)` / `165=Extract(29,0)` / `166=OpFOrdLessThan(165,24)` / four `OpSelect(166,…)`.

⇒ Rules:

- **Dump first** (`--nocapture`, `export_spirv`, an example that prints the bytes), **then** write the matcher against the dump.
- **Give every artifact criterion a reverse self-check**: flip the threshold (e.g. `1.0` ⇒ must go red), or replace the operand with a constant (⇒ must go red). Without it, "matched the wrong thing" is indistinguishable from "correctly matched".

### Pick the right *level* of criterion before making one complicated

Ask first: **"which layer of equivalence am I proving?"** Then choose the criterion. If a criterion has to get complicated to pass, that is usually a signal you picked the wrong layer — not that the artifact is wrong.

**Case (three versions, the first two refuted by measurement).** To show "the refactored shader is equivalent to the original": v1 — "byte-identical after a pure `Id` renumbering" — went **red** (330 words differed), so the differences were **not** just `Id`s. v2 added "pin `OpLoad` results to reserved slots" and still went **red** (133 words): "renumber by first-appearance order" is inherently a **sequence** concept, so any reordering shifts everything downstream — **the wrong tool for the job**. v3 switched to a **constant-time structural-signature multiset** and went green, with a reverse self-check (changing a single constant ⇒ must go red; measured red).

Same author also hit both wrong ways of the underlying fixed-point iteration: **string-concatenating** sub-keys made the key space blow up so it never converged; switching to a **fixed-length hash** made it order-dependent and it still never converged.

⇒ Rules:

- **Word-by-word / byte-by-byte comparison cannot prove graph isomorphism** (equivalence up to renumbering and ordering). Structural equivalence needs a **structural** criterion — and such a criterion is typically **simple and constant-time**.
- **Every equivalence-class criterion must state what it cannot prove.** In the case above the author wrote explicitly: "this does not catch wiring permutations; the authoritative evidence remains the 20663-point oracle + GPU pixel comparison." A criterion with no stated blind spot will be trusted beyond its power.

### After restoring a file in place, clean the crate or prove a rebuild

`git checkout --` / `Copy-Item` / any in-place restore can leave a **stale `target/`**: restored files may keep their old `mtime`, or incremental compilation may reuse the **mutated `.o`**. Your next measurement then tests the old artifact, not the restored source.

**Case.** Restoring `spirv.rs` with `Copy-Item` from a backup **preserved the mtime**, so cargo did not rebuild; the resulting run led to the (completely wrong) conclusion that "the discriminant is not in the library", and a long detour. Sibling incident: a restore from backup left tests red because incremental compilation reused the mutated object file.

⇒ Rules:

- After any in-place restore, run **`cargo clean -p <crate>`**, or **confirm in the output that it really recompiled** (watch for `Compiling <crate>`).
- **`git status` being clean does not mean `target/` is clean.** A clean worktree says nothing about what the compiler will reuse.

## Design constraints and known traps

These are not suggestions. Each one exists because a defect was found, and each is guarded by a test. Read this section before changing layout, rendering, or text code.

### Layout invariants

Asserted in `crates/deer-layout/tests/layout_invariants.rs`, one test per invariant:

| # | Invariant |
|---|---|
| I-1 | **Pure** — does not mutate the input tree; returns only the geometry table |
| I-2 | **Deterministic** — identical input yields bit-identical output (no time, randomness, or environment probing) |
| I-3 | **Bottom-up** — child intrinsic sizes are computed before the parent allocates |
| I-4 | **Pixel-rounded** — all geometry is integral |
| I-5 | **No window ownership assumed** — the root box is given by the host; the root does not stretch to fill |
| I-6 | **No overflow** — results are clamped to the available space |
| I-7 | **Allocated size ≠ available space** — a parent's main-axis allocation must be honored; percentages resolve against the parent's content box |
| I-8 | **Main axis = sum(children), cross axis = max(children)** |

I-7 and I-8 were both established **after** being burned by real defects.

### The five real defects kept as regression guards

The TypeScript prototype (V0) found three by breaking the code on purpose and watching the tests go red:

| # | Defect | Guard |
|---|---|---|
| **B-1** | The two authoring paths used different id rules (a single counter vs per-type counting), so the trees were not equal | `t1_two_authoring_paths_produce_the_same_tree`, `t1_auto_id_rule_is_shared` |
| **B-2** | Layout treated a parent's allocation as an upper bound on available space, so `grow` allocations were silently dropped | `t5_grow_fills_the_row` |
| **B-3** | Container intrinsic size ignored explicit child sizes, with the wrong direction semantics | `t13`, `t14_container_cross_axis_is_max_not_sum` |

Two more were found during the Rust port:

- **R-1 (construction order)** — `with_props` / `with_layout` are **whole-value assignments**; setting them after an id is chosen overwrites the id too.
- **R-2 (alignment vs grow)** — computing main-axis alignment remainder **before** `grow` means that once `grow` takes effect, `center` / `end` fail **silently**.

### Text and measurement

- Layout, draw lists, and rasterization must use **the same font size and the same metrics**. `ApproxMeasure` is retained only as a deterministic test implementation.
- Only `glyf` outlines are supported. CFF / OpenType-CFF (`OTTO`) fonts are rejected at parse time — deliberately, rather than silently returning empty outlines.
- Hinting, subpixel positioning, kerning and ligatures (`kern` / `GSUB` / `GPOS`), vertical text, and RTL are **not implemented**.

### Vulkan

- **Static viewport/scissor** (offscreen path, an implementation fact — not a driver limitation). The old claim that "dynamic state drew nothing on an Intel integrated GPU" is **contested/unverified**: dynamic and static both render 93900 UI pixels over 30 frames on that same iGPU, while declaring dynamic state *without ever calling* `vkCmdSetViewport` crashes (`0xC000041D` window / `0xC0000005` offscreen). M2a's recorded symptom was "no pixels", not a crash, so "same root cause" is only *likely*. Do not call `vkCmdSetViewport` / `vkCmdSetScissor` on the static pipeline (validation error); create a new renderer when the extent changes. See `docs/features/window.md` §5.2.
- **The color attachment must be `R8G8B8A8_UNORM`, not `_SRGB`.** The CPU baseline applies no gamma conversion, so SRGB introduces a systematic difference. The window swapchain follows the same rule (linear `*_UNORM` first) because sRGB attachments blend in linear space (~44-byte difference vs the CPU's byte-space blend).
- **`spirv.rs::vertex_shader_rect_pushconstant` is broken — do not build a pipeline with it.** The validation layer reports `VUID-StandaloneSpirv-PushConstant-06808`, and requesting the layer makes the process crash with `0xc0000005`. Ordinary drivers accept it leniently, which is exactly why "the driver accepted it" is not a correctness argument. Rectangles go through vertex buffers instead.
- **Hand-written SPIR-V must go through `spirv-val`.** The historical root cause of "the driver neither errors nor draws" was a section-ordering bug. `vkCreateShaderModule` is very permissive.
- The device, offscreen, and windowed paths all honor `DEER_VK_VALIDATION` now; do not explain test behavior using the older assumption that the offscreen path ignored it.

## Commits and pull requests

Commit messages follow Conventional Commits with a Chinese description; the milestone usually appears in the scope or body:

```
feat(deer-vk): offscreen GPU geometry renderer (static pipeline + vertex buffer + readback)
fix(deer-vk): clamp out-of-range alpha to the CPU baseline
docs(m3a): GPU geometry guide + gpu_geometry example + FEATURES/ROADMAP/TUTORIAL reconciliation
```

For a bug fix, describe the **root cause** in the body, not only the lines you changed.

Before opening a pull request:

- the smallest relevant gate passes (see [Verification](#verification)), and you state which one you ran;
- new behavior ships with its example, guide, and `FEATURES.md` entry;
- line endings are LF (`.gitattributes` enforces `eol=lf`; CRLF breaks byte-level reproducibility);
- you did not hand-edit `Cargo.lock` — change `Cargo.toml` and let cargo update it;
- if you added a dependency, it is **registered in `ROADMAP.md` with a rationale**.

Please state in the PR what you verified, in what environment, and what you did **not** verify. Skipping or omitting that is the most common cause of review churn here.

## What not to do

- **Do not weaken a check to get a green run.** Deleting tests, relaxing thresholds, replacing real assertions with `assert!(true)`, or turning failures into skips all count as cheating. If you cannot run something, say "not verified in this environment".
- **Do not silently add dependencies.** `deer-layout`, `deer-gpu`, `deer-vk`, and `deer-gui` without the `window` feature must stay free of third-party dependencies. Graphics abstraction libraries (`wgpu`, `ash`, `vulkano`, `glow`) and GUI frameworks (`egui`, `iced`, `tauri`) are explicitly out of scope.
- **Do not write "fully zero-dependency."** The correct phrasing is "no third-party dependencies except the windowing layer (`winit`, registered)".
- **Do not quote numbers from memory.** Test and assertion counts change every milestone; use what the run printed.
- **Do not refactor opportunistically.** Keep a change to the step it needs.
