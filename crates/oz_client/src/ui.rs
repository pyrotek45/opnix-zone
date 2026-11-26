//! A tiny widget kit: panels, labels and buttons that work the same with a
//! mouse, a keyboard or a gamepad.
//!
//! Screens are rebuilt from scratch whenever [`Screen`] changes or
//! [`UiDirty`] is set, which keeps every screen a plain function of state.
//! Buttons carry an [`Act`]; activating one (click, Enter, or A) emits an
//! [`ActEvent`] that the owning module handles.

use crate::{ClientSet, fx::PlaySfx, input::Controls};
use oz_sim::state::LobbyAction;
use bevy::{ecs::hierarchy::ChildSpawnerCommands, prelude::*};
use oz_sim::prelude::*;

pub const TEXT: Color = Color::srgb(1.0, 0.92, 0.72);
pub const DIM: Color = Color::srgb(0.56, 0.63, 0.62);
pub const ACCENT: Color = Color::srgb(0.45, 0.88, 0.95);
pub const WARN: Color = Color::srgb(1.0, 0.45, 0.36);
pub const GOOD: Color = Color::srgb(0.55, 1.0, 0.65);
const PANEL: Color = Color::srgba(0.025, 0.05, 0.075, 0.95);
const BUTTON: Color = Color::srgba(0.08, 0.13, 0.17, 0.95);
const BUTTON_FOCUS: Color = Color::srgb(0.24, 0.30, 0.26);
const BORDER: Color = Color::srgb(0.30, 0.40, 0.42);
const BORDER_FOCUS: Color = Color::srgb(1.0, 0.80, 0.40);

/// Which full-screen UI is open. `None` means "playing".
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub enum Screen {
    #[default]
    Main,
    Join,
    Settings,
    None,
    Pause,
    Station { kind: u16, page: usize },
    /// Versus class editor.
    Loadout,
    /// Versus pre-match lobby.
    Lobby,
    /// Weapon customisation with a 3D preview.
    Gunsmith { weapon: u8, slot: u8, from: Back },
    /// Every robot pose side by side.
    Lineup,
    /// Typing a chat message (the game keeps running behind it).
    Chat,
    GameOver,
}

/// Where the gunsmith returns to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Back {
    Loadout,
    Station { kind: u16, page: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Sensitivity,
    AdsSensitivity,
    Fov,
    NetStats,
    InvertY,
    PixelSize,
    Dither,
    Shadows,
    Fullscreen,
    Vsync,
    Volume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Callsign,
    Address,
}

/// Everything a button can do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Go(Screen),
    Back,
    /// Previous / next tab (bumpers, PageUp / PageDown).
    Tab(i32),
    // Front end.
    Solo,
    Host,
    Connect,
    Mode(i32),
    Biome(i32),
    Difficulty(i32),
    TeamSize(i32),
    Set(Setting, i32),
    Edit(Field),
    Quit,
    Leave,
    // Stations.
    Order(PlayerCommand),
    Place(u16),
    Page(usize),
    Weapon(i32),
    // Loadout: which field (0 primary, 1 secondary, 2 perk, 3 gear) and the step.
    Class(u8, i32),
    Deploy,
    // Gunsmith.
    OpenGunsmith(u8, Back),
    GunSlot(u8),
    Part(u8, u8),
    // Lobby.
    Lobby(LobbyAction),
    /// Join a game found on the local network (index into the finder's list).
    JoinFound(usize),
    FindGames,
    ToggleSpectate,
}

/// The action of the button that currently has focus (hover, keyboard or pad).
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct Focused(pub Option<Act>);

#[derive(Event, Clone, Debug)]
pub struct ActEvent(pub Act);

#[derive(Component)]
pub struct Clickable {
    pub act: Act,
    /// Action for "left" on a value button; "right" and click use `act`.
    pub left: Option<Act>,
    order: u32,
}

#[derive(Component)]
pub struct ScreenRoot;

/// Set to rebuild the current screen next frame.
#[derive(Resource, Default)]
pub struct UiDirty(pub bool);

/// Index of the focused button, in layout order.
#[derive(Resource, Default)]
pub struct Focus(pub usize);

/// Hands out layout order to buttons as a screen is built.
#[derive(Default)]
pub struct Order(u32);

pub struct UiKitPlugin;

impl Plugin for UiKitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Screen>()
            .init_resource::<Focus>()
            .init_resource::<Focused>()
            .insert_resource(UiDirty(true))
            .add_event::<ActEvent>()
            .add_systems(Update, (mark_dirty, interact).chain().in_set(ClientSet::Input).after(crate::input::ReadDevices));
    }
}

fn mark_dirty(screen: Res<Screen>, mut dirty: ResMut<UiDirty>, mut focus: ResMut<Focus>, mut last: Local<Option<Screen>>) {
    if screen.is_changed() {
        dirty.0 = true;
        // Moving between pages of a station keeps the cursor; a new screen resets it.
        let same_family = matches!((&*last, &*screen), (Some(Screen::Station { .. }), Screen::Station { .. }));
        if !same_family && last.as_ref() != Some(&*screen) {
            focus.0 = 0;
        }
        *last = Some(screen.clone());
    }
}

/// Removes the old screen. Screen builders run right after this when dirty.
pub fn clear_screen(mut commands: Commands, dirty: Res<UiDirty>, roots: Query<Entity, With<ScreenRoot>>) {
    if dirty.0 {
        for root in &roots {
            commands.entity(root).despawn();
        }
    }
}

pub fn finish_rebuild(mut dirty: ResMut<UiDirty>) {
    dirty.0 = false;
}

/// Full-screen centred container. `dim` darkens the game behind it.
pub fn root<'a>(commands: &'a mut Commands, dim: f32) -> EntityCommands<'a> {
    commands.spawn((
        ScreenRoot,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(8.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.01, 0.02, dim)),
        GlobalZIndex(10),
    ))
}

/// Full-screen container with its content on the left, leaving the right for the showroom.
pub fn root_left<'a>(commands: &'a mut Commands, dim: f32) -> EntityCommands<'a> {
    commands.spawn((
        ScreenRoot,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            justify_content: JustifyContent::Center,
            padding: UiRect::left(Val::Percent(5.0)),
            row_gap: Val::Px(8.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.01, 0.02, dim)),
        GlobalZIndex(10),
    ))
}

pub fn panel(width: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(width),
            max_width: Val::Percent(96.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            padding: UiRect::all(Val::Px(18.0)),
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor(BORDER),
    )
}

pub fn row(gap: f32) -> impl Bundle {
    Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(gap), align_items: AlignItems::Center, flex_wrap: FlexWrap::Wrap, row_gap: Val::Px(gap), ..default() }
}

pub fn label(parent: &mut ChildSpawnerCommands, text: impl Into<String>, size: f32, color: Color) {
    parent.spawn((Text::new(text), TextFont { font_size: size, ..default() }, TextColor(color)));
}

fn button_node(width: Val) -> impl Bundle {
    (
        Button,
        Node { width, padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)), border: UiRect::all(Val::Px(2.0)), ..default() },
        BackgroundColor(BUTTON),
        BorderColor(BORDER),
    )
}

/// A full-width button.
pub fn button(parent: &mut ChildSpawnerCommands, order: &mut Order, text: impl Into<String>, act: Act) {
    sized_button(parent, order, text, act, None, Val::Percent(100.0));
}

/// A value button: left / right change the value, click advances it.
pub fn stepper(parent: &mut ChildSpawnerCommands, order: &mut Order, text: impl Into<String>, left: Act, right: Act) {
    sized_button(parent, order, text, right, Some(left), Val::Percent(100.0));
}

pub fn sized_button(parent: &mut ChildSpawnerCommands, order: &mut Order, text: impl Into<String>, act: Act, left: Option<Act>, width: Val) {
    order.0 += 1;
    parent.spawn((button_node(width), Clickable { act, left, order: order.0 })).with_children(|button| label(button, text, 17.0, TEXT));
}

/// A button that keeps its place in the focus order but paints itself as selected.
pub fn chip(parent: &mut ChildSpawnerCommands, order: &mut Order, text: impl Into<String>, act: Act, selected: bool, width: Val) {
    order.0 += 1;
    let color = if selected { GOOD } else { TEXT };
    parent.spawn((button_node(width), Clickable { act, left: None, order: order.0 })).with_children(|button| label(button, text, 15.0, color));
}

/// A greyed-out row that cannot be focused.
pub fn disabled(parent: &mut ChildSpawnerCommands, text: impl Into<String>, width: Val) {
    parent
        .spawn((Node { width, padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)), border: UiRect::all(Val::Px(2.0)), ..default() }, BorderColor(Color::srgba(0.3, 0.4, 0.42, 0.35))))
        .with_children(|row| label(row, text, 17.0, DIM));
}

/// Focus, hover, click and gamepad / keyboard activation for every button on screen.
fn interact(
    controls: Res<Controls>,
    mut focus: ResMut<Focus>,
    mut buttons: Query<(&Interaction, &Clickable, &mut BackgroundColor, &mut BorderColor)>,
    changed: Query<(&Interaction, &Clickable), Changed<Interaction>>,
    mut acts: EventWriter<ActEvent>,
    mut sounds: EventWriter<PlaySfx>,
    mut focused: ResMut<Focused>,
) {
    let mut orders: Vec<u32> = buttons.iter().map(|(_, clickable, ..)| clickable.order).collect();
    if orders.is_empty() {
        if focused.0.is_some() {
            focused.0 = None;
        }
        return;
    }
    orders.sort_unstable();
    let rank = |order: u32| orders.binary_search(&order).unwrap_or(0);
    let before = focus.0.min(orders.len() - 1);
    let mut now = before;
    let mut fired = false;

    for (interaction, clickable) in &changed {
        match interaction {
            Interaction::Hovered => now = rank(clickable.order),
            Interaction::Pressed => {
                now = rank(clickable.order);
                acts.write(ActEvent(clickable.act.clone()));
                fired = true;
            }
            Interaction::None => {}
        }
    }
    let nav = controls.ui;
    if nav.down {
        now = (now + 1) % orders.len();
    }
    if nav.up {
        now = (now + orders.len() - 1) % orders.len();
    }
    if let Some((_, clickable, ..)) = buttons.iter().find(|(_, clickable, ..)| clickable.order == orders[now]) {
        if nav.accept || (nav.right && clickable.left.is_some()) {
            acts.write(ActEvent(clickable.act.clone()));
            fired = true;
        } else if nav.left {
            if let Some(left) = &clickable.left {
                acts.write(ActEvent(left.clone()));
                fired = true;
            }
        }
    }
    if nav.back {
        acts.write(ActEvent(Act::Back));
    }
    if nav.tab_prev || nav.tab_next {
        acts.write(ActEvent(Act::Tab(nav.tab_next as i32 - nav.tab_prev as i32)));
    }
    if fired {
        sounds.write(PlaySfx::ui(Sfx::UiAccept));
    } else if now != before {
        sounds.write(PlaySfx::ui(Sfx::UiMove));
    }
    focus.0 = now;
    let current = buttons.iter().find(|(_, clickable, ..)| clickable.order == orders[now]).map(|(_, clickable, ..)| clickable.act.clone());
    if focused.0 != current {
        focused.0 = current;
    }

    for (_, clickable, mut background, mut border) in &mut buttons {
        let focused = clickable.order == orders[now];
        let (fill, edge) = if focused { (BUTTON_FOCUS, BORDER_FOCUS) } else { (BUTTON, BORDER) };
        if background.0 != fill {
            background.0 = fill;
            border.0 = edge;
        }
    }
}
