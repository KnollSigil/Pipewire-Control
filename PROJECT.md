# Project: Pipewire Control Usability Features

## Architecture
Pipewire Control is a Rust desktop application built with `iced` 0.14 for managing PipeWire audio/video routing graphs.
- **`src/model.rs`**: Core graph data structures (`PwGraph`, `PwNode`, `PwPort`, `PwLink`, `CollapsedEdge`), presets, routing matrix computations, and demo graph generator (`create_demo_graph()`).
- **`src/main.rs`**: Application state (`PipewireApp`), event loop, toolbar, category filters, auto-layout engine, overlap-nudging solver, inspector panel, and subscription handling.
- **`src/canvas_view.rs`**: Custom Iced `Canvas` widget (`GraphCanvas`, `Program`), bezier wire drawing, node card rendering, sockets, tooltips, interaction state (`CanvasInteraction`), drag gestures, and selection.
- **`src/backend.rs`**: Asynchronous PipeWire client loop (`pipewire_subscription`) communicating via `tokio::sync::mpsc` (`PwEvent`) and `pw::channel` (`PwCommand`). Manages live proxies (`Node`, `Port`, `Link`, `Device`), SPA pod deserialization/serialization, and fallback demo mode.

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | R7.1 Remove Nudge Apart | Remove "Nudge Apart" button and message handler; automatic overlap nudging on drag stays | M1 | ORIGINAL_REQUEST §R7 |
| 2 | R7.2 Conditional Refresh Button | "Refresh PipeWire" button hidden when connected/healthy, visible only when disconnected/failed; triggers reconnect/sync | M1 | ORIGINAL_REQUEST §R7 |
| 3 | R3 Decluttered Link Badges | Mini routing-matrix badge drawn only when `!is_default \|\| is_hovered \|\| is_selected` | M2 | ORIGINAL_REQUEST §R3 |
| 4 | R2.1 Link Tooltip Sizing | Tooltip width sized proportionally to fit label text without clipping or excess margin | M2 | ORIGINAL_REQUEST §R2 |
| 5 | R2.2 Link Tooltip Positioning | Tooltip anchored near cursor position instead of midpoint, clamped within canvas bounds | M2 | ORIGINAL_REQUEST §R2 |
| 6 | R1.1 Node Search Input | Search box in top bar filtering by name and description case-insensitively | M3 | ORIGINAL_REQUEST §R1 |
| 7 | R1.2 Node Search Dimming | Non-matching nodes and their links dimmed (not hidden); layout and positions unchanged; clearing restores normal rendering | M3 | ORIGINAL_REQUEST §R1 |
| 8 | R4.1 Signal Flow Columns | Nodes arranged left-to-right by signal flow depth: sources at col 0, sinks fed by col 0 + unconnected sinks at col 1, sinks fed by col 1 at col 2, etc. | M4 | ORIGINAL_REQUEST §R4 |
| 9 | R4.2 Link Crossing Minimization | Barycentric vertical ordering within columns to keep connected nodes vertically aligned | M4 | ORIGINAL_REQUEST §R4 |
| 10 | R4.3 Link Collision Avoidance | No node placed on top of links it is not an endpoint of (bezier curve clearance) | M4 | ORIGINAL_REQUEST §R4 |
| 11 | R4.4 Layout Invariant & Stability | Full layout runs only on startup and Auto-Layout button; node/link changes and volume/mute never move existing nodes | M4 | ORIGINAL_REQUEST §R4 |
| 12 | R4.5 Free-Spot Placement | Newly added nodes placed in sensible free spot without overlapping or moving existing nodes | M4 | ORIGINAL_REQUEST §R4 |
| 13 | R5.1 Multi-Select Gestures | Shift/Ctrl click toggles selection; marquee box drag on empty canvas selects contained nodes; click empty canvas clears | M5 | ORIGINAL_REQUEST §R5 |
| 14 | R5.2 Group Drag & Pinned Nudge | Dragging any selected node moves the entire group by identical delta; overlap nudging treats group as pinned (never nudges group members against each other) | M5 | ORIGINAL_REQUEST §R5 |
| 15 | R6.1 Device Routes/Profiles Model | Model device profiles and routes with active, available, and unavailable states | M6 | ORIGINAL_REQUEST §R6 |
| 16 | R6.2 Demo Backend Routes/Profiles | Demo nodes (Scarlett Solo, Built-in Audio) provide realistic routes/profiles; switching active state; rejecting unavailable | M6 | ORIGINAL_REQUEST §R6 |
| 17 | R6.3 Live PipeWire Device API | Bind `pipewire::device::Device` proxies, subscribe to Profile/Route params, set via `device.set_param(...)` with SPA pods (zero CLI shell out) | M6 | ORIGINAL_REQUEST §R6 |
| 18 | R6.4 Inspector Device UI | Inspector displays readable routes and profiles, visually distinguishing active, available, and unavailable entries | M6 | ORIGINAL_REQUEST §R6 |
| 19 | Screenshot Harness & Script | Repeatable test/script rendering demo mode to PNG files for scenarios (a)-(h) and reviewer visual validation | M7 | ORIGINAL_REQUEST §Visual verification |
| 20 | Final Review & Integrity Audit | Point-by-point independent code review, full regression suite, clippy clean check, and forensic audit | M8 | ORIGINAL_REQUEST §Acceptance Criteria |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | Toolbar Cleanup (R7) | Remove Nudge Apart; conditional Refresh PipeWire; unit tests; git commit `debc9df` | None | DONE |
| M2 | Link Badges & Tooltips (R2, R3) | Decluttered mini badges; proportional tooltip sizing; cursor anchoring & canvas clamping; unit tests; git commit `ebc24f1` | M1 | DONE |
| M3 | Node Search Filter (R1) | Search input; case-insensitive name/description matching; canvas dimming; position stability; unit tests; git commit `fc60b16` | M2 | DONE |
| M4 | Signal Flow Auto-Layout (R4) | Signal flow depth columns; barycentric crossing minimization; bezier collision clearance; free-spot allocator; stability; unit tests; git commit `eca6d34` | M3 | DONE |
| M5 | Multi-Selection & Group Drag (R5) | HashSet selection; Shift/Ctrl toggle; marquee box selection; delta group drag; pinned overlap nudging; unit tests; git commit `773e4d5` | M4 | DONE |
| M6 | Device Routes & Profiles (R6) | Data model; demo graph & backend; live PipeWire Device proxy & SPA pods; inspector UI; unit tests | M5 | DONE |
| M7 | Visual Verification Harness | Script & harness for scenarios (a)-(h); capture PNGs; reviewer visual check; commit script | M6 | PLANNED |
| M8 | Final Acceptance & Audit | Complete test suite verification (pre-existing 14 + all new); clippy zero new warnings; independent review; forensic audit | M7 | PLANNED |

## Interface Contracts

### M1: Toolbar & Health
- `PipewireApp::is_connection_healthy(&self) -> bool`: returns `!self.backend.is_demo && self.backend.cmd_sender.is_some()`.
- Toolbar renders Refresh button only if `!self.is_connection_healthy()`.
- `Message::NudgeApart` deleted; "Nudge Apart" button removed from `view_top_bar()`.

### M2: Badges & Tooltips
- `canvas_view::should_show_link_badge(is_default: bool, is_hovered: bool, is_selected: bool) -> bool`: returns `!is_default || is_hovered || is_selected`.
- `canvas_view::compute_tooltip_width(label: &str, text_size: f32) -> f32`: computes tight proportional width + padding.
- `canvas_view::compute_tooltip_rect(label: &str, text_size: f32, anchor: Point, bounds: Rectangle) -> Rectangle`: anchors near pointer and clamps inside `bounds`.

### M3: Search Filter
- `model::matches_search(name: &str, description: &str, query: &str) -> bool`: case-insensitive substring search.
- `PipewireApp::search_query: String`: bound to search `text_input` in top bar.
- `canvas_view::GraphCanvas` receives `search_query: &str` or `dimmed_serials: &HashSet<u64>`. Non-matching nodes and non-matching wires rendered with dimmed opacity.

### M4: Auto-Layout
- `compute_signal_flow_columns(graph: &PwGraph, visible_nodes: &HashSet<u64>) -> BTreeMap<u64, usize>`: computes column indices by signal flow depth.
- `node_intersects_link_curve(node_rect: Rectangle, p0: Point, p3: Point) -> bool`: sampled cubic bezier collision test.
- `find_sensible_free_spot(graph: &PwGraph, existing_positions: &BTreeMap<u64, Point>, new_node: &PwNode) -> Point`: locates non-overlapping, non-crossing position.
- Full layout runs only on startup (`node_positions.is_empty()`) and `Message::AutoLayout`.

### M5: Multi-Selection
- `PipewireApp::selected_nodes: HashSet<u64>` replaces single `Option<u64>` (helper `selected_node()` returns single serial if `selected_nodes.len() == 1`).
- `CanvasInteraction::marquee_rect: Option<Rectangle>` and `marquee_start: Option<Point>`.
- `resolve_node_overlaps_pinned(positions: &mut BTreeMap<u64, Point>, pinned: &HashSet<u64>)`: group members never nudge each other.

### M6: Device Routes & Profiles
- `PwDeviceProfile { index: i32, name: String, description: String, priority: u32, available: PwAvailability, active: bool }`
- `PwDeviceRoute { index: i32, name: String, description: String, direction: PortDirection, available: PwAvailability, active: bool, device_id: Option<i32>, profiles: Vec<i32> }`
- `PwCommand::SetDeviceProfile { device_serial: u64, profile_index: i32 }`
- `PwCommand::SetDeviceRoute { device_serial: u64, route_index: i32, device_id: i32 }`
- `Backend::set_device_profile` and `Backend::set_device_route`: live SPA pod serialization + demo mode mutation.

## Code Layout
- `src/main.rs`: Top bar UI, search state, multi-select state, inspector device UI, layout & nudging solver.
- `src/canvas_view.rs`: Wire rendering, tooltips, badge visibility, marquee box, group drag, dimming.
- `src/model.rs`: Graph structures, route & profile definitions, search matching, demo graph.
- `src/backend.rs`: PipeWire device proxy listener, param enumeration, SPA pod handling, demo device state.
- `tests/`: Programmatic unit and integration tests.
- `scripts/`: Visual screenshot generation script.
