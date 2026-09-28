# deer-gui

A GUI runtime for Rust, written from scratch — no web, no DOM, and no `wgpu` / `ash` / `vulkano`.

[![Rust](https://img.shields.io/badge/rust-1.85%2B-blue.svg)](#requirements)
[![Edition](https://img.shields.io/badge/edition-2024-blue.svg)](#requirements)
[![Dependencies](https://img.shields.io/badge/third--party%20deps-none%20(except%20winit)-green.svg)](#dependencies)
[![Platform](https://img.shields.io/badge/window%20layer-Windows-lightgrey.svg)](#platform-support)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**English** | [中文](README.zh-CN.md)

Licensed under MIT. No third-party dependencies except the windowing layer (`winit`, [registered in the roadmap](ROADMAP.md)).

---

## Overview

deer-gui is a GUI runtime built around a **node tree**: you describe a UI, the tree is laid out by a layout engine written in this repository, and the result is handed to a pluggable render backend.

Two authoring paths produce **the same tree**:

- an **imperative API** with an imgui-like feel (`Builder`), and
- a **scene file** format (`.dui`, in the spirit of Godot's `.tscn`).

The layout algebra comes from a TypeScript validation prototype (which ran 28 assertions at the time — a historical value of that prototype, not a current test count for this repository); the widget vocabulary comes from `deer-ui`. This is **not** a port of a web project — DOM and CSS are discarded entirely, and the window, input, and rendering layers are implemented here.

> [!NOTE]
> The project is under active development. `FEATURES.md` is the single source of truth for what works today, what is partial, and what is not implemented yet. Please read it before assuming a feature exists.

## Status

| Area | Status |
|---|---|
| `deer-layout` — node tree, layout algebra, hit testing, `.dui` scene parsing | ✅ One test per layout invariant in `crates/deer-layout/tests/layout_invariants.rs` |
| `deer-gpu` — GPU HAL + CPU reference backend (software rasterizer) | ✅ Contract in place; the CPU backend rasterizes draw lists to pixels and can blit real glyphs offscreen |
| `deer-vk` — Vulkan backend (symbols declared by hand, loaded at runtime) | ✅ Enumerates 2 GPUs on real hardware (Intel RaptorLake / NVIDIA RTX 5070 Ti, Vulkan 1.4.341); device, pipeline, and offscreen readback, plus surface / swapchain / present |
| `deer-window` — windowing layer (winit) | ✅ Real window and event loop on **Windows**; the only third-party dependency in the workspace |
| Real glyphs — font parsing, rasterization, atlas, real metrics, line breaking | ✅ Offscreen on the CPU, and on the GPU as a coverage-texture pass (`R8_UNORM` atlas, nearest sampling, compared pixel-for-pixel against the CPU backend). Hinting and subpixel positioning are still not done |
| On-screen rendering in a window | ✅ Shapes and text are presented through a shared pipeline layer on a linear swapchain, compared pixel-for-pixel against the CPU backend. Event dispatch and focus are still M5 |
| Batch optimization (segment merging, cross-frame buffer reuse) | 🔄 Segment merging into one `vkCmdDraw` and persistent vertex buffers have landed; the pipeline-switch count and the unified / single-buffer variants are still open |
| Input events, focus, dockable panels | ⬜ Not implemented (milestones M5/M6) |

See [`FEATURES.md`](FEATURES.md) for the per-feature breakdown and [`ROADMAP.md`](ROADMAP.md) for milestones.

## Requirements

- **Rust 1.85 or newer** (the workspace uses edition 2024).
- **Windows** for the windowing layer and anything that presents to a window. Everything else — layout, CPU rasterization, offscreen rendering to PNG, and the Vulkan offscreen path — builds and runs without a window.
- **No Vulkan SDK is required.** `deer-vk` declares the Vulkan symbols itself and loads `vulkan-1.dll` at runtime through `LoadLibraryW` + `GetProcAddress`, so only `kernel32` is linked. Struct layouts are hand-written and pinned with `offset_of!` assertions.

## Installation

The crates are not published yet; use the workspace directly:

```sh
git clone https://github.com/DeerLuuu/deer-gui.git
cd deer-gui
```

To consume the library from another project, depend on it by path:

```toml
[dependencies]
deer-gui = { path = "path/to/deer-gui/crates/deer-gui" }
```

> [!IMPORTANT]
> Add `--features window` only if you need a real window. Without it, `winit` is not pulled in and `deer-gui` has no third-party dependencies.

## Quick start

```sh
# Run the whole assertion suite (the count grows with each milestone — read it from the output)
cargo test --workspace

# See which GPUs are enumerated on this machine
cargo test -p deer-vk -- --nocapture

# Render an image offscreen (CI-friendly) -> render_out/render_to_png.png
cargo run -p deer-gui --example render_to_png

# Render real glyphs to PNG -> render_out/text_render.png
cargo run -p deer-gui --example text_render

# Open a real window (Windows only; the feature flag is required)
cargo run -p deer-gui --features window --example window_preview

# End-to-end check: window pixels vs the CPU backend (gated; prints "this is a skip, not a pass" if the gate is unset)
DEER_VK_WINDOW_TESTS=1 cargo run -p deer-gui --features window --example window_parity
```

If you have never written Rust, start with the more verbose [getting-started guide](docs/GETTING-STARTED.md).

### Usage

```rust
use deer_gui::prelude::*;
use deer_gui::render_tree_to_png;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build the tree (or parse a scene file: parse_scene(text, "ui.dui")?)
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("Title");
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("OK");
        r.button("Cancel");
    });
    let tree = app.build();

    // 2. Render it to a PNG
    std::fs::write("ui.png", render_tree_to_png(&tree, 320, 200, Theme::default())?)?;
    Ok(())
}
```

- Want pixels instead of a file? `render_tree_to_rgba(&tree, w, h, theme)` returns `(width, height, RGBA8)`.
- Want to inspect layout only? `deer_gui::layout_tree(&tree, w, h, theme)` returns a geometry table from node id to `Rect`.

### What rendering can and cannot do today

| Capability | Status |
|---|---|
| Build a tree (imperative API / `.dui` scene file) | ✅ |
| Layout computation and hit testing | ✅ |
| Tree + geometry → draw list → **pixels** (CPU rasterizer) | ✅ |
| Write PNG files | ✅ |
| **Real glyphs** (font parsing → rasterization → atlas → pixels) | ✅ offscreen / CPU backend |
| **GPU-side text** (glyph quads + `R8_UNORM` atlas texture, nearest sampling) | ✅ compared byte-for-byte against the CPU backend |
| **Rendering into a window on screen** (shapes and text) | ✅ pixel-compared against the CPU backend via `window_parity` |
| Input events and focus | ❌ Not implemented |

One subtlety worth knowing: the older entry point `render_tree_to_png` does not load a font, so text drawn through it is still evenly spaced placeholder boxes. Real glyphs come from the text engine (`text_render`). Geometry, layering, and color are real on both paths.

## Architecture

```
crates/
├── deer-layout/   Platform-independent core: node tree, layout algebra, hit testing, .dui parsing
│                  ── zero platform dependencies, fully assertable in a CI without a GPU
├── deer-gpu/      GPU HAL: Backend/Device/Swapchain/Frame traits, DrawList, CPU reference backend
│                  ── adding a backend means implementing one trait
├── deer-vk/       Vulkan backend: hand-declared extern symbols + runtime loading; surface/swapchain/present
└── deer-window/   Windowing: native window + event loop (winit)
                   ── the only place a third-party dependency is introduced (winit);
                      it hands the renderer an opaque RawWindowHandle, so swapping the window
                      implementation does not touch the render layers
```

### Two authoring paths, one tree

```text
  Builder (imperative, imgui-like) ─┐
                                    ├─→ Node tree ─→ layout() → geometry ─→ backend
  parse_scene (.dui, tscn-like)    ─┘               └─→ hit_test() → input routing
```

Both paths produce **structurally equal** trees (`Node::structurally_eq`). That is the core invariant, and it is pinned by the test `t1_two_authoring_paths_produce_the_same_tree`.

```text
# The equivalent scene file (.dui)
[column name=app pad=12 gap=8]
  [text label=Title]
  [row name=bar gap=8]
    [button label=OK]
    [button label=Cancel]
```

### Layout invariants

Each invariant has a test in `crates/deer-layout/tests/layout_invariants.rs`:

| # | Invariant |
|---|---|
| I-1 | **Pure**: does not mutate the input tree, returns only the geometry table |
| I-2 | **Deterministic**: identical input yields bit-identical output (no time, randomness, or environment probing) |
| I-3 | **Bottom-up**: child intrinsic sizes are computed before the parent allocates |
| I-4 | **Pixel-rounded**: all geometry is integral |
| I-5 | **Does not assume window ownership**: the root box is given by the host, and the root does not stretch to fill |
| I-6 | **No overflow**: results are clamped to the available space |
| I-7 | **Allocated size ≠ available space**: a parent's main-axis allocation must be honored; percentages resolve against the parent's content box |
| I-8 | **Main axis = sum(children), cross axis = max(children)**: a container's intrinsic size depends on direction |

### Design constraints and known traps

Behavior that looks surprising is usually deliberate. The full list, with the defects that produced each rule and the tests that guard them, lives in [CONTRIBUTING.md](CONTRIBUTING.md#design-constraints-and-known-traps):

- Layout, draw lists, and rasterization must use **the same font size and the same metrics**.
- Vulkan uses a **static viewport/scissor** (an implementation fact, not a driver limitation). The old claim
  that "dynamic viewport draws nothing on some integrated GPUs" has been **refuted on this machine** (offscreen +
  triangle, three-way control): dynamic *with* a per-frame `vkCmdSetViewport` is byte-identical to static (0 validation
  messages, pixels drawn), while dynamic *without ever setting it* crashes (`0xC0000005` offscreen / `0xC000041D` window).
  Boundaries: this is a **local measurement** (not "dynamic is now supported" across devices), and M2a recorded
  "no pixels without crashing" — a *different symptom* — so the original cause is still unexplained.
  Offscreen keeps using static (C1 only refuted the *reason*); see `docs/features/window.md` §5.2.
- The color attachment must be `R8G8B8A8_UNORM`, **not** `_SRGB`, because the CPU baseline does no gamma conversion.
  The window swapchain follows the same rule (linear `*_UNORM` first): sRGB attachments blend in linear space,
  which differs from the CPU's byte-space `blend_cov` by ~44 bytes.
- Five real defects found during validation (three inherited from the prototype, two found in the Rust port) are kept as regression guards.

## Documentation

| Document | What it covers |
|---|---|
| [Tutorial](docs/TUTORIAL.md) | Step by step, 14 sections (§0–§13); every section runs on its own |
| [Feature list](FEATURES.md) | **Which features exist, how far they are, and which are not done** — the single source of truth |
| [Per-feature guides](docs/features/) | Complete usage and pitfalls for one feature at a time |
| [Roadmap](ROADMAP.md) | Milestones M1–M7 and their acceptance criteria |
| [Getting started](docs/GETTING-STARTED.md) | A slower on-ramp if you have never written Rust |
| [Agent notes](agent.md) | Repository rules and traps for AI agents and new maintainers |

### Crate-level examples

The [`crates/deer-gui/examples/`](crates/deer-gui/examples/) directory is the fastest way to see a feature working — 15 runnable examples, each with a self-check assertion:

```sh
cargo run -p deer-gui --example tutorial        # guided tour, writes render_out/*.png
cargo run -p deer-gui --example geometry        # layout and hit testing
cargo run -p deer-gui --example scene_file      # .dui authoring, equivalent tree
cargo run -p deer-gui --example gpu_geometry    # GPU geometry, compared against the CPU backend
cargo run -p deer-gui --example glyph_atlas     # glyph atlas packing
```

See [`FEATURES.md`](FEATURES.md) for the example that belongs to each feature.

## Dependencies

Except for the windowing layer, the workspace has **no third-party dependencies**:

| Crate | Third-party dependencies |
|---|---|
| `deer-layout` | none |
| `deer-gpu` | none |
| `deer-vk` | none |
| `deer-gui` (without the `window` feature) | none |
| `deer-window`, `deer-gui --features window` | `winit 0.30` |

`winit` is the single registered exception, recorded in [`ROADMAP.md`](ROADMAP.md) under Q-1. New dependencies must be registered there with a rationale before being added.

## Platform support

| Platform | Layout, CPU rasterization, PNG output | Vulkan offscreen | Window / present |
|---|---|---|---|
| Windows | ✅ | ✅ (Vulkan 1.4, driver-provided) | ✅ |
| Linux / macOS | ✅ | ✅ | ❌ returns an unsupported-platform error |

## Contributing

This repository treats documentation as part of the deliverable: a new feature is not complete until it ships with an example, a guide, and an entry in `FEATURES.md`. The rules, the review gates, and the command reference are in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — the full text is in [LICENSE](LICENSE); the same identifier is declared in the `license` field of [`Cargo.toml`](Cargo.toml).
