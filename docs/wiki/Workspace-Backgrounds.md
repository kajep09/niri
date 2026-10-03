# Workspace backgrounds (local implementation)

Background layer-shell surfaces can be restricted to one workspace. Image
loading and surface creation remain the wallpaper client's responsibility.

```kdl
workspace "work"
layer-rule {
    match namespace="^my-work-background$"
    place-on-workspace "work"
}
```

Names are case-insensitive, like other Niri workspace names. For an unnamed or
dynamic workspace, use its stable `id` from `niri msg --json workspaces`:

```kdl
layer-rule {
    match namespace="^my-workspace-42-background$"
    place-on-workspace-id 42
}
```

An ID is **not** an output-local workspace index. IDs remain attached when a
workspace is reordered or renamed, and are valid only for the current compositor
session. Generating/reloading rules for newly created workspaces is supported;
there is no automatic namespace interpretation or image-management policy.

## Semantics

- Rules restrict **background** layers only. Other layers keep their ordinary
  behavior. `place-within-backdrop true` takes precedence and remains output-wide.
- Untargeted backgrounds keep their existing behavior. Scoped and unscoped
  surfaces keep the normal layer stacking order; a scoped image does not
  implicitly hide other background surfaces.
- The last matching rule that specifies a name or ID replaces the earlier
  target. If both are present in the same rule, the ID wins.
- Unknown/deleted workspace targets render nowhere and do not create or keep a
  workspace alive. A name target follows that name if it is later reassigned;
  an ID target never follows a replacement workspace.
- Layer-shell output ownership is unchanged. A surface appears only if its
  target workspace is on its own output. Moving a workspace to another output
  hides the old surface; clients can create the corresponding surface on the new
  output. Output removal follows the existing layer-shell close behavior.
- Backgrounds move and clip with their workspace in transitions, gestures and
  Overview. Pointer and keyboard targeting respect workspace selection.
- Xray/blur uses separate buffers for visible workspaces that have scoped
  backgrounds; ordinary workspaces share the existing output-wide buffer.
  Workspace buffers are released after leaving the visible set. Normal,
  screenshot and screencast targets stay separate, including unmap snapshots.
- Existing presentation-based frame throttling applies. Off-workspace surfaces
  are not marked presented merely because another workspace uses blur.

## Local verification

The expanded implementation was visually verified in nested Niri: distinct
backgrounds during switching and Overview, a transparent floating GTK window
with Xray/blur showing the correct background, and ID targeting surviving a
workspace rename after a config reload. These solid-color checks establish blur
source selection, not detailed convolution quality. The regular compositor,
config, IPC and documentation tests pass, including a workspace-selection and
Xray-cache lifecycle integration test. Additional integration tests cover popup
grabs, output moves/removal, rule reloads, deleted workspace IDs, hidden-surface
presentation tracking and GPU-rendered capture blocking through unmap snapshots.
A nested checkerboard comparison verifies actual blur smoothing. Sustained
animated-client performance and real DRM/multi-output sessions remain deployment
checks.

Relevant upstream context: [discussion #2350](https://github.com/niri-wm/niri/discussions/2350),
[Overview PR #1440](https://github.com/niri-wm/niri/pull/1440),
[Blur PR #3483](https://github.com/niri-wm/niri/pull/3483).
This local feature does not depend on upstream accepting it.
