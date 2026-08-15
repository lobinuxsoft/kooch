# Frame Pacing

A game loop is supposed to spin. An editor showing a still image is not.

Until #656 the engine made no distinction: the winit handler asked for
the next redraw at the end of every frame, unconditionally, so the loop
fed itself forever. Vsync capped it at the refresh rate, which is the
only reason it cost one core per process rather than all of them. Idle,
with a project open and nothing happening, that measured at two pinned
cores and 51.8 W on a 9800X3D — to display an image that was not
changing.

## The contract

Two types in `kooch_core::frame_pacing`:

- **`FrameRequest`** — what *this* frame decided the next one needs.
  Systems raise it; the runner reads it once per frame and resets it to
  a baseline. Raising is monotonic within a frame: the most urgent
  request wins, so draw order can never talk a system out of a repaint
  it asked for.
- **`FrameWaker`** — a clonable handle any thread can use to break the
  loop out of a sleep. The wake is sticky, so one that lands between the
  end of a frame and the moment the runner commits to sleeping is not
  lost.

Three paces, in order of urgency:

| Pace | Means | `ControlFlow` |
|---|---|---|
| `Continuous` | Something is animating or simulating | `Poll` + `request_redraw` |
| `After(d)` | Something is on a timer | `WaitUntil(now + d)` |
| `Wait` | Nothing to draw | `Wait` |

**An app that inserts no `FrameRequest` keeps spinning.** That is not an
oversight — it is what a shipped game wants, and it means the opt-in is
explicit at every call site that needs it.

## Who asks for what

**The editor** (`FrameRequest::new(FramePace::Wait)`) takes its answer
from egui, which already computes one: `run_ui` returns a
`repaint_delay` per viewport, `ZERO` while something animates and
`Duration::MAX` when the UI has drawn everything it has. Two things egui
cannot see are folded in on top:

- **Play** — the viewport texture changes from another process, with no
  widget to notice it through. `Continuous` for as long as Play lasts.
- **A live remote session** — the project's stdout arrives on a socket,
  not as a window event, so a fully asleep editor would hold its Console
  output until the user happened to move the mouse. `After(250 ms)`.

A frame that failed to present asks for another unconditionally: what is
on screen is not what that frame drew.

**A project under an editor** (`RemotePlugin`) sleeps by default and is
woken by its own socket. Between edits nothing simulates, so a frame
nobody asked for is a core spent mirroring a still scene; `Playing`
raises the pace for as long as Play lasts. The listener thread parks on
a reply only the main thread can produce, which is why the wake is not
optional — without it, an editor asking a perfectly healthy project a
question would hang until something unrelated produced a frame.

## Frames stopped being a clock

Anything that said "every N frames" was reading a clock that no longer
ticks at a fixed rate. An idle editor draws roughly four frames a
second, so "every thirtieth frame" went from half a second to seven and
a half.

The remote snapshot pull was the one such cadence in the tree, and it is
now expressed as a `Duration`. **Any new cadence should be too** — frame
counts were always a stand-in for time, and they are no longer even a
good one.

## Input never waits

While the loop is idle, an input event is the only thing that will
produce a frame, so every window event other than `RedrawRequested` asks
for one. A `WaitUntil` deadline expiring reports through
`StartCause::ResumeTimeReached`, and a cross-thread wake arrives as a
winit user event — the proxy rather than `request_redraw`, because the
proxy is the API documented to be callable from another thread.

## The swapchain image is asked for last

The game runtime's frame is two halves that need very different things.
The meshlet stage — cull, raster, shading, shadows, TAA, tonemap — draws
into textures the engine owns and submits its own command buffer. Only
the sky and the blit write to the surface.

So the surface image is acquired **between** them, and not before both:

```text
record + submit the scene   ──►  get_current_texture()  ──►  sky, blit, present
        ~34 ms of GPU work         blocks on the compositor      ~0.65 ms
```

🔴 **Acquiring first costs a full frame of overlap**, and that is what
this did until #837. `get_current_texture` blocks until the presentation
engine releases an image, so asking for it before recording puts the
whole CPU-side of the frame *after* the wait — and the GPU cannot start
this frame's work until the compositor has let go of the last one.
Measured on the OneXFly: a median frame of 37.14 ms made of 34 ms of GPU
and 3.006 ms of recording, added together rather than overlapped.

Nothing about the image is needed to record the scene. The dependency
was in the control flow, not in the data.

⚠️ **The editor already did this correctly**, which is why the two paths
look different: `systems/present.rs` tessellates the UI, uploads its
textures and updates its buffers before acquiring, and the viewport
passes run earlier still. Only the game runtime had the acquire on top.

### What this does not fix

The frame-time distribution is bimodal on the OneXFly — the same GPU
work produces a 34.7 ms frame and a 69.4 ms one — and this change does
not address that.

Two explanations were on the table and **both are now refuted**, by three
30-second captures of one binary with one variable changed each:

| | latency 2 | latency 3 | `novsync` |
|---|---|---|---|
| frame/GPU p80 | 1.98 | 1.99 | 1.94 |
| frame/GPU p90 | 2.49 | 2.50 | 2.21 |
| `vkAcquireNextImageKHR` ms/frame | 35.209 | 33.646 | 37.162 |

A third swapchain image does not move the ratio by a hundredth. Neither
does leaving FIFO.

🔴 **An acquire of ~35 ms against a GPU of ~35 ms is not a defect.**
Being GPU-bound means the CPU waits somewhere, and `get_current_texture`
is where. Reading that number as a symptom is a mistake this document
used to make. What is genuinely unexplained is only the **tail**: the
frames where the wait grows by 50 ms while our own GPU work grows by 2.

⚠️ **A present mode is close to decorative when a compositor owns the
display.** These captures run under gamescope, which composites on the
same GPU, on its own schedule, and is **invisible to our scopes** — they
time our passes and nothing else. `novsync` turns off *our* vsync, not
its. Whatever is left lives outside this process, and no environment
variable on this side is going to find it.

The next measurement is not another engine knob. It is gamescope's own
frame statistics, or a run without gamescope at all.

### And what the tail was hiding

```text
GPU:          ~35 ms
budget:        13.9 ms
```

**2.5x over, with a still camera and at half shading rate.** If the tail
vanished entirely the frame would still miss by more than double, and
`shade: compute (half rate)` alone — 19.7 to 22.8 ms across the three
captures — costs more than the whole frame is allowed.

The tail is a mystery in 30% of frames. The shading is 60% of every one
of them.
