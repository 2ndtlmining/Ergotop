# Ergotop — Physics Block Building (Design Spec)

Date: 2026-10-03
Status: Draft for review
Builds on: `2026-10-03-ergotop-rust-design.md` §4.2 (packing visualizer) and §4.4 (animation)

## 1. Goal

Make the next-block visualizer feel physical while keeping today's packing, colors and sizes:

- new txs fall with gravity (accelerating) and land with a small bounce;
- repacked txs slide sideways / drop into new spots instead of jumping;
- a "water" fill line shows how full the next block is;
- when a block is mined, its txs launch upward together and the remaining txs avalanche into the gap;
- txs that leave without being mined fade out.

### Non-goals

- New views, mempool rain, whale alerts, flow network, history charts.
- Real physics simulation (collisions, stacking dynamics).
- Changing the packing algorithm, block selection, sprite sizes or colors.

### Constraints

- Full Dashboard frame with 10,000 txs, all mid-animation: < 5 ms (today 1.43 ms static).
- Every animation is a pure function of time (deterministic, testable at exact timestamps).
- Position is continuous: no sprite ever teleports while motion is on.

## 2. Approach: time-based tweens

Every movement is a `Tween { from: (f32, f32), to: (f32, f32), start_ms: u64, dur_ms: u64, ease: Ease }`. A sprite's on-screen position at time `t` is `tween.pos(t)`; before `start_ms` it is `from`, after `start_ms + dur_ms` it is `to`. A new move always starts from the sprite's current `pos(now)`, so interrupting a move mid-way never jumps.

Coordinates are visualizer pixels, origin bottom-left (as today).

### 2.1 Curves (`Ease`)

| Ease | Shape | Used for |
|---|---|---|
| `Gravity` | quadratic ease-in for the fall (`p = u²`), then a bounce tail: after reaching the target the sprite rises by `min(0.15 · fall_height, 2 px)` and settles back, the tail taking 250 ms of the duration | new tx drop, drop-only repack, avalanche |
| `Slide` | cubic ease-in-out on both axes | repack with a horizontal change |
| `Launch` | quadratic ease-in upward (accelerating) | mined txs leaving |
| `Linear` | linear | fill-line level changes (with 500 ms duration) |

### 2.2 Timings

| Constant | Value |
|---|---|
| Full-height gravity fall | 1,200 ms (duration for a fall of distance `d` = `1200 · sqrt(d / height)` ms, minimum 120 ms) + 250 ms bounce tail |
| Slide | 400 ms |
| Mined flash | 400 ms (alternating white / tx color every 100 ms) |
| Launch | 700 ms, from current position to `height + side` (fully off the top) |
| Avalanche stagger | delay = `400 · x / width` ms (left columns first), capped at 400 ms |
| Fade (dropped) | 300 ms: full color → half brightness → removed |
| Pending hold | up to 20,000 ms (longer than the reconciler's 15 s drop grace) |
| Fill-line glide | 500 ms |

## 3. Behaviour

### 3.1 Sprite states

`Arriving` (tween running) · `Resting` · `Pending` (left the pool, outcome unknown) · `Flashing` · `Launching` · `Fading`.

### 3.2 Events

| Event (from `App`) | Visualizer reaction |
|---|---|
| Relayout, new id | spawn above the block at its packed x (`y = height`), `Gravity` drop to its packed y |
| Relayout, existing id, same target | no change |
| Relayout, existing id, target lower, same x | `Gravity` drop from current position |
| Relayout, existing id, x changed (or target higher) | `Slide` from current position |
| Id left the pool (present in viz, absent from pool) | becomes `Pending`: stays in place, drawn at half brightness, **still occupies its packed space**; excluded from the title counts |
| `Mined` for a pending or resting id | `Flashing` 400 ms → `Launching` 700 ms → removed; then relayout without it; other sprites that move get `Gravity` drops with the avalanche stagger |
| `Dropped` for a pending id, or pending for 20 s | `Fading` 300 ms → removed; then relayout (avalanche as above) |
| Resync / resize / motion off | everything placed instantly (no tweens), pending/flashing/launching/fading cleared |

Pending sprites keep their space by staying in the packing input (as items flagged `pending`) until their outcome is known; this is what lets mined txs launch from where they were built and the avalanche happen after the launch.

### 3.3 Fill line

Level (pixel row) = `block_bytes_excluding_pending / capacity · block_height`, eased with `Linear` over 500 ms when it changes. Drawn behind sprites as a wave: pixel `(x, level + w(x, t))` with `w ∈ {0, 1}` alternating along x and shifting phase every 150 ms, in the theme's accent color at half brightness. Hidden when the level is 0. The dotted capacity line at the top of the block region stays.

### 3.4 Motion toggle

`m` toggles motion at runtime (status message "Motion on/off"); `[ui] motion = true|false` in `ergotop.toml` sets the default (true). With motion off, all events place sprites instantly and the fill line jumps; mined/dropped sprites disappear at once. Help (`?`) lists `m`.

## 4. Code structure

- **New `crates/ergotop/src/anim.rs`** — pure: `Ease`, `Tween { pos(now), done(now) }`, constructors `gravity_drop(from, to, start, height)`, `slide(from, to, start)`, `launch(from, top, start)`, `glide(from, to, start)`, and `avalanche_delay(x, width) -> u64`. No state, no I/O.
- **`viz.rs`** — sprites hold `state` + `tween`; `relayout(items, capacity, now)` chooses tweens per §3.2 (`VizItem` gains `pending: bool`); `on_mined(ids, now)`, `on_dropped(ids, now)`, `expire_pending(now)`; `set_motion(bool)`; `tick(now) -> bool` (true while any tween/flash/fade/launch or fill glide is active); `render(canvas, now, theme colors)` computes positions from tweens and draws the fill wave. The current `Phase`, `FALL_PX_PER_S`, `LEAVE_PX_PER_S` and the parked-sprite mechanism are replaced.
- **`app.rs`** — keeps a `leaving: HashMap<TxId, (VizItem, since_ms)>` for ids that left the pool; passes pool items + leaving items (`pending: true`) to `relayout`; routes `Mined`/`Dropped` to the visualizer and drops ids from `leaving`; `m` key; reads `[ui] motion`.
- **`ergotop-core` config** — `UiConfig.motion: bool` (serde default true).
- **`ui/packing.rs`** — title counts exclude pending; passes theme accent to `render`.

## 5. Testing

- **Tween math (`anim.rs`)**: exact positions — gravity at `t=start` equals `from`, at end of fall equals `to`, bounce peak ≤ 2 px and ≤ 15% of fall, final position equals `to`; slide midpoint at half duration; launch accelerates (distance in 2nd half > 1st half) and ends above the top; avalanche delays monotonic in x and ≤ 400 ms.
- **Visualizer**: continuity (position before and after a mid-fall relayout differ by < 1 px); sideways repack produces a slide, not a jump; pending sprite keeps its packed slot and is excluded from counts; mined → flash, launch, gone by flash+launch, then avalanche starts left before right; dropped and 20 s timeout → fade → gone after 300 ms; fill level reaches target after 500 ms; motion off places instantly.
- **App**: leaving ids become pending and resolve on `Mined`/`Dropped`; `m` toggles; config default true.
- **Snapshots**: packing view at a fixed mid-animation time.
- **Benchmark**: Dashboard frame with 10,000 txs all mid-tween < 5 ms; render-only visualizer bench with active tweens.
