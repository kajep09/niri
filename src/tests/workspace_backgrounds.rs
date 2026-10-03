//! Generic layer-shell coverage, independent of any wallpaper client.
use niri_config::{Config, WorkspaceReference};
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};
use wayland_client::protocol::wl_surface::WlSurface;

use super::client::{ClientId, LayerConfigureProps};
use super::Fixture;
use crate::niri::Niri;
use smithay::reexports::wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use wayland_client::{Connection, Dispatch, QueueHandle};

#[derive(Default)]
struct PopupEvents {
    dismissed: AtomicBool,
}

impl Dispatch<xdg_popup::XdgPopup, Arc<PopupEvents>> for super::client::State {
    fn event(
        _: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        data: &Arc<PopupEvents>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, xdg_popup::Event::PopupDone) {
            data.dismissed.store(true, Ordering::Relaxed);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, Arc<PopupEvents>> for super::client::State {
    fn event(
        _: &mut Self,
        proxy: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &Arc<PopupEvents>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
        }
    }
}

wayland_client::delegate_noop!(super::client::State: xdg_positioner::XdgPositioner);

fn config(rule: &str) -> Config {
    Config::parse(
        std::path::Path::new("workspace-backgrounds.kdl"),
        &format!(
            r#"
        hotkey-overlay {{ skip-at-startup; }}
        workspace "one"
        workspace "two"
        layer-rule {{
            match namespace="^scoped$"
            {rule}
        }}
    "#
        ),
    )
    .config
    .unwrap()
}

fn fixture(rule: &str) -> (Fixture, ClientId) {
    let mut f = Fixture::with_config(config(rule));
    f.add_output(1, (800, 600));
    let client = f.add_client();
    (f, client)
}

fn map_layer(
    f: &mut Fixture,
    id: ClientId,
    output: u8,
    interactivity: KeyboardInteractivity,
) -> WlSurface {
    // The registry roundtrip binds outputs; their name events arrive afterward.
    f.double_roundtrip(id);
    let wl_output = f.client(id).output(&format!("headless-{output}"));
    let layer = f
        .client(id)
        .create_layer(Some(&wl_output), Layer::Background, "scoped");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Top | Anchor::Bottom),
        kb_interactivity: Some(interactivity),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(800, 600);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);
    surface
}

fn cache_ids(f: &mut Fixture, output: u8) -> Vec<u64> {
    f.niri_complete_animations();
    let output = f.niri_output(output);
    f.niri().update_xray_render_elements(Some(&output));
    let mut ids: Vec<_> = f.niri().output_state[&output]
        .xray
        .workspace_backgrounds
        .keys()
        .copied()
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn output_move_does_not_reassign_layer_output_and_removal_closes_it() {
    let (mut f, client) = fixture("place-on-workspace \"one\"");
    f.add_output(2, (800, 600));
    let surface = map_layer(&mut f, client, 1, KeyboardInteractivity::None);
    f.niri_focus_output(1);
    f.niri().layout.switch_workspace(0);
    let wsid = f.niri().layout.active_workspace().unwrap().id().get();
    assert_eq!(cache_ids(&mut f, 1), [wsid]);
    let output2 = f.niri_output(2);
    assert!(f.niri().layout.move_workspace_to_output(&output2));
    assert!(cache_ids(&mut f, 1).is_empty());
    assert!(
        cache_ids(&mut f, 2).is_empty(),
        "an output-1 surface must not follow the workspace"
    );
    let surface2 = map_layer(&mut f, client, 2, KeyboardInteractivity::None);
    assert_eq!(cache_ids(&mut f, 2), [wsid]);

    f.niri().remove_output(&output2);
    f.double_roundtrip(client);
    assert!(f.client(client).layer(&surface2).close_requested);
    assert!(!f.client(client).layer(&surface).close_requested);
    f.niri_focus_output(1);
    let idx = f
        .niri()
        .layout
        .workspaces()
        .find(|(_, _, ws)| ws.id().get() == wsid)
        .unwrap()
        .1;
    f.niri().layout.switch_workspace(idx);
    assert_eq!(cache_ids(&mut f, 1), [wsid]);
}

#[test]
fn live_rule_reload_retargets_and_removes_workspace_restriction() {
    let (mut f, client) = fixture("place-on-workspace \"one\"");
    map_layer(&mut f, client, 1, KeyboardInteractivity::None);
    let wsid = f
        .niri()
        .layout
        .find_workspace_by_name("two")
        .unwrap()
        .1
        .id()
        .get();
    f.niri_state()
        .reload_config(Ok(config(&format!("place-on-workspace-id {wsid}"))));
    assert!(cache_ids(&mut f, 1).is_empty());
    f.niri().layout.switch_workspace(1);
    assert_eq!(cache_ids(&mut f, 1), [wsid]);
    f.niri().layout.set_workspace_name("renamed".into(), None);
    f.niri().layout.move_workspace_to_idx(None, 0);
    assert_eq!(cache_ids(&mut f, 1), [wsid]);
    f.niri_state().reload_config(Ok(config("")));
    assert!(cache_ids(&mut f, 1).is_empty());
    let niri = f.niri();
    let mapped = niri.mapped_layer_surfaces.values().next().unwrap();
    assert!(Niri::layer_visible_on_workspace(mapped, None));
}

#[test]
fn scoped_surface_does_not_keep_deleted_workspace_alive_or_target_replacement() {
    let (mut f, client) = fixture("");
    let wsid = f
        .niri()
        .layout
        .find_workspace_by_name("two")
        .unwrap()
        .1
        .id()
        .get();
    f.niri_state()
        .reload_config(Ok(config(&format!("place-on-workspace-id {wsid}"))));
    map_layer(&mut f, client, 1, KeyboardInteractivity::None);
    f.niri()
        .layout
        .unname_workspace_by_ref(WorkspaceReference::Id(wsid));
    f.niri_complete_animations();
    assert!(!f
        .niri()
        .layout
        .workspaces()
        .any(|(_, _, ws)| ws.id().get() == wsid));
    assert!(cache_ids(&mut f, 1).is_empty());
    // Reintroducing the same name creates a distinct ID and cannot reuse the old target.
    f.niri_state()
        .reload_config(Ok(config(&format!("place-on-workspace-id {wsid}"))));
    let replacement = f
        .niri()
        .layout
        .find_workspace_by_name("two")
        .unwrap()
        .1
        .id()
        .get();
    assert_ne!(replacement, wsid);
    f.niri().layout.switch_workspace(1);
    assert!(cache_ids(&mut f, 1).is_empty());
}

#[test]
fn exclusive_background_loses_keyboard_eligibility_when_hidden() {
    let (mut f, client) = fixture("place-on-workspace \"one\"");
    map_layer(&mut f, client, 1, KeyboardInteractivity::Exclusive);
    f.niri_state().update_keyboard_focus();
    let niri = f.niri();
    let mapped = niri.mapped_layer_surfaces.values().next().unwrap();
    let surface = mapped.surface().wl_surface().clone();
    assert_eq!(niri.keyboard_focus.surface(), Some(&surface));
    f.niri().layout.switch_workspace(1);
    f.niri_complete_animations();
    f.niri_state().update_keyboard_focus();
    assert_ne!(f.niri().keyboard_focus.surface(), Some(&surface));
    f.niri().layout.switch_workspace(0);
    f.niri_complete_animations();
    f.niri_state().update_keyboard_focus();
    assert_eq!(f.niri().keyboard_focus.surface(), Some(&surface));
}

fn center_pixel(f: &mut Fixture, target: crate::render_helpers::RenderTarget) -> Vec<u8> {
    use crate::render_helpers::{render_to_vec, RenderCtx};
    use smithay::backend::allocator::Fourcc;
    use smithay::utils::Transform;
    let output = f.niri_output(1);
    f.niri().update_render_elements(Some(&output));
    let state = f.niri_state();
    state
        .backend
        .headless()
        .with_primary_renderer(|renderer| {
            let elements = state.niri.render_to_vec(
                RenderCtx {
                    renderer,
                    target,
                    xray: None,
                },
                &output,
                false,
            );
            let pixels = render_to_vec(
                renderer,
                (800, 600).into(),
                1.0.into(),
                Transform::Normal,
                Fourcc::Abgr8888,
                elements.iter().rev(),
            )
            .unwrap();
            pixels[(300 * 800 + 400) * 4..(300 * 800 + 400) * 4 + 3].to_vec()
        })
        .unwrap()
}

#[test]
fn egl_scoped_xray_respects_capture_targets_and_unmap_snapshot() {
    use crate::render_helpers::RenderTarget::{Output, ScreenCapture, Screencast};
    let mut cfg = config("place-on-workspace \"one\"\nblock-out-from \"screen-capture\"");
    let window_config = Config::parse(
        std::path::Path::new("windows.kdl"),
        r#"
        window-rule {
            open-floating true
            draw-border-with-background false
            background-effect {
                xray true
                blur true
            }
        }
    "#,
    )
    .config
    .unwrap();
    cfg.window_rules = window_config.window_rules;
    let mut f = Fixture::with_config(cfg);
    f.niri_state().backend.headless().add_renderer().unwrap();
    f.add_output(1, (800, 600));
    let client = f.add_client();
    let layer_surface = map_layer(&mut f, client, 1, KeyboardInteractivity::None);
    let layer = f.client(client).layer(&layer_surface);
    let buffer =
        layer
            .spbm
            .create_u32_rgba_buffer(u32::MAX, u32::MAX, u32::MAX, u32::MAX, &layer.qh, ());
    layer.surface.attach(Some(&buffer), 0, 0);
    layer.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
    layer.commit();
    let window = f.client(client).create_window();
    let window_surface = window.surface.clone();
    window.commit();
    f.roundtrip(client);
    let window = f.client(client).window(&window_surface);
    window.attach_new_buffer();
    window.set_size(400, 300);
    window.ack_last_and_commit();
    f.double_roundtrip(client);
    f.niri_complete_animations();

    let pixel = center_pixel(&mut f, Output);
    assert!(pixel.iter().all(|&c| c > 245), "output pixel: {pixel:?}");
    assert!(center_pixel(&mut f, ScreenCapture).iter().all(|&c| c < 10));
    assert!(center_pixel(&mut f, Screencast).iter().all(|&c| c < 10));
    // Rendering a restricted target must not poison the ordinary output buffer.
    assert!(center_pixel(&mut f, Output).iter().all(|&c| c > 245));

    // Capture the window's closing snapshot with its Xray background embedded.
    // Freeze time so that the animation remains present during both render calls.
    f.niri().clock.set_rate(0.0);
    f.client(client).window(&window_surface).attach_null();
    f.client(client).window(&window_surface).commit();
    f.double_roundtrip(client);
    assert!(center_pixel(&mut f, Output).iter().all(|&c| c > 245));
    assert!(center_pixel(&mut f, Screencast).iter().all(|&c| c < 10));
}

#[test]
fn egl_hidden_background_is_not_marked_presented_for_frame_callbacks() {
    use crate::render_helpers::{create_texture, RenderCtx, RenderTarget};
    use smithay::backend::allocator::Fourcc;
    use smithay::backend::renderer::{damage::OutputDamageTracker, Bind};
    use smithay::desktop::utils::surface_primary_scanout_output;
    use smithay::utils::Transform;
    use smithay::wayland::compositor::with_states;

    let (mut f, client) = fixture("place-on-workspace \"one\"");
    f.niri_state().backend.headless().add_renderer().unwrap();
    map_layer(&mut f, client, 1, KeyboardInteractivity::None);
    let output = f.niri_output(1);
    let mut damage = OutputDamageTracker::new((800, 600), 1.0, Transform::Normal);
    for (index, visible) in [(0, true), (1, false), (0, true)] {
        f.niri().layout.switch_workspace(index);
        f.niri_complete_animations();
        f.niri().update_render_elements(Some(&output));
        let state = f.niri_state();
        state
            .backend
            .headless()
            .with_primary_renderer(|renderer| {
                let elements = state.niri.render_to_vec(
                    RenderCtx {
                        renderer,
                        target: RenderTarget::Output,
                        xray: None,
                    },
                    &output,
                    false,
                );
                let mut texture =
                    create_texture(renderer, (800, 600).into(), Fourcc::Abgr8888).unwrap();
                let mut target = renderer.bind(&mut texture).unwrap();
                let result = damage
                    .render_output(renderer, &mut target, 0, &elements, [0.; 4])
                    .unwrap();
                state
                    .niri
                    .update_primary_scanout_output(&output, &result.states);
            })
            .unwrap();
        let mapped = f.niri().mapped_layer_surfaces.values().next().unwrap();
        let surface = mapped.surface().wl_surface();
        let primary = with_states(surface, |states| {
            surface_primary_scanout_output(surface, states)
        });
        assert_eq!(
            primary.is_some(),
            visible,
            "off-workspace layer must use fallback rather than refresh-rate frame callbacks"
        );
    }
}

#[test]
fn pointer_only_popup_is_dismissed_on_switch_and_hidden_grab_is_rejected() {
    let (mut f, client) = fixture("place-on-workspace \"one\"");
    let layer_surface = map_layer(&mut f, client, 1, KeyboardInteractivity::None);
    let create_popup = |f: &mut Fixture| {
        let client = f.client(client);
        let surface = client.state.create_surface();
        let shell = client.state.xdg_wm_base.as_ref().unwrap();
        let events = Arc::new(PopupEvents::default());
        let xdg = shell.get_xdg_surface(&surface, &client.qh, events.clone());
        let positioner = shell.create_positioner(&client.qh, ());
        positioner.set_size(40, 40);
        positioner.set_anchor_rect(0, 0, 40, 40);
        let popup = xdg.get_popup(None, &positioner, &client.qh, events.clone());
        client
            .state
            .layers
            .iter()
            .find(|l| l.surface == layer_surface)
            .unwrap()
            .layer_surface
            .get_popup(&popup);
        let seat = client.state.seats.keys().next().unwrap();
        popup.grab(seat, 1);
        surface.commit();
        (surface, xdg, popup, events)
    };
    let (_surface, _xdg, _popup, events) = create_popup(&mut f);
    f.double_roundtrip(client);
    let grab = f
        .niri()
        .popup_grab
        .as_ref()
        .expect("eligible background must acquire a popup grab");
    assert!(!grab.has_keyboard_grab);
    assert!(f.niri().seat.get_pointer().unwrap().is_grabbed());
    f.niri().layout.switch_workspace(1);
    f.niri_state().update_keyboard_focus();
    f.double_roundtrip(client);
    assert!(f.niri().popup_grab.is_none());
    assert!(!f.niri().seat.get_pointer().unwrap().is_grabbed());
    assert!(events.dismissed.load(Ordering::Relaxed));

    let (_surface2, _xdg2, _popup2, events2) = create_popup(&mut f);
    f.double_roundtrip(client);
    assert!(f.niri().popup_grab.is_none());
    assert!(events2.dismissed.load(Ordering::Relaxed));
}
