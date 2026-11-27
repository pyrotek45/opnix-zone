//! A tiny widget kit: panels, labels and buttons that work the same with a
//! mouse, a keyboard or a gamepad.
//!
//! Screens are rebuilt from scratch whenever [`Screen`] changes or
//! [`UiDirty`] is set, which keeps every screen a plain function of state.
//! Buttons carry an [`Act`]; activating one (click, Enter, or A) emits an
//! [`ActEvent`] that the owning module handles.

use crate::{ClientSet, fx::PlaySfx, input::Controls};
use bevy::{ecs::hierarchy::ChildSpawnerCommands, prelude::*};
use oz_sim::prelude::*;
use oz_sim::state::LobbyAction;

pub const TEXT: Color = Color::srgb(1.0, 0.92, 0.72);
pub const DIM: Color = Color::srgb(0.56, 0.63, 0.62);
pub const ACCENT: Color = Color::srgb(0.45, 0.88, 0.95);
pub const WARN: Color = Color::srgb(1.0, 0.45, 0.36);
pub const GOOD: Color = Color::srgb(0.55, 1.0, 0.65);
const PANEL: Color = Color::srgba(0.025, 0.05, 0.075, 0.95);
const BUTTON: Color = Color::srgba(0.08, 0.13, 0.17, 0.95);
const BUTTON_FOCUS: Color = Color::srgb(0.24, 0.30, 0.26);
const BORDER: Color = Color::srgb(0.30, 0.40, 0.42);
const BUTTON_EDIT: Color = Color::srgb(0.16, 0.34, 0.22);
const BORDER_FOCUS: Color = Color::srgb(1.0, 0.80, 0.40);

/// Which full-screen UI is open. `None` means "playing".
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub enum Screen {
    #[default]
    Main,
    Join,
    Settings,
    /// Key bindings.
    Controls,
    None,
    Pause,
    Station {
        kind: u16,
        page: usize,
    },
    /// Versus class editor.
    Loadout,
    /// Versus pre-match lobby.
    Lobby,
    /// Weapon customisation with a 3D preview.
    Gunsmith {
        weapon: u8,
        slot: u8,
        from: Back,
    },
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
    /// Weather choice (0 = the map's own).
    Weather(i32),
    /// Versus map layout choice.
    Layout(i32),
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
    // Armory (class editor).
    /// Choose which of the five classes to edit (lobby) or spawn with (in a match).
    ClassPick(u8),
    /// Primary, secondary, perk or gear.
    ArmoryTab(u8),
    /// Put this weapon / perk / gear (catalog index) in the current tab's slot.
    ArmoryItem(u8),
    /// Step the attachment in this slot of the current tab's weapon.
    ArmoryPart(u8, i32),
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
    /// Wait for a new key for this action (index into `bindings::Action::ALL`).
    Rebind(u8),
    ResetBindings,
    /// Try the host we lost again.
    Reconnect,
}

/// Ask for focus to land on the button with this action once it exists (screens use
/// it to move the cursor between columns, e.g. back to a list with B).
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct FocusRequest(pub Option<Act>);

/// Where a button sits on a screen's grid (column, row), for d-pad / arrow navigation.
#[derive(Component, Clone, Copy, Debug)]
pub struct GridPos(pub IVec2);

/// The value button being edited (its layout order), if any.
#[derive(Resource, Default)]
pub struct Editing(pub Option<u32>);

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
            .init_resource::<FocusRequest>()
            .init_resource::<Editing>()
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
    (Button, Node { width, padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)), border: UiRect::all(Val::Px(2.0)), ..default() }, BackgroundColor(BUTTON), BorderColor(BORDER))
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

/// A button at a place on the screen's grid (`col`, `row`): the d-pad moves along rows and
/// between them. `left` makes it a value button (A to edit, then left / right).
#[allow(clippy::too_many_arguments)]
pub fn grid_button(parent: &mut ChildSpawnerCommands, order: &mut Order, text: impl Into<String>, act: Act, left: Option<Act>, width: Val, col: i32, row: i32) {
    order.0 += 1;
    parent.spawn((button_node(width), Clickable { act, left, order: order.0 }, GridPos(IVec2::new(col, row)))).with_children(|button| label(button, text, 17.0, TEXT));
}

/// A slim button for long lists (the armory): less padding, smaller type.
pub fn slim(parent: &mut ChildSpawnerCommands, order: &mut Order, text: impl Into<String>, act: Act, left: Option<Act>, selected: bool, width: Val) {
    slim_at(parent, order, text, act, left, selected, width, None);
}

/// [`slim`] at a place on the screen's grid (`col`, `row`).
#[allow(clippy::too_many_arguments)]
pub fn slim_at(parent: &mut ChildSpawnerCommands, order: &mut Order, text: impl Into<String>, act: Act, left: Option<Act>, selected: bool, width: Val, cell: Option<IVec2>) {
    order.0 += 1;
    let color = if selected { GOOD } else { TEXT };
    let mut button = parent.spawn((
        Button,
        Node { width, padding: UiRect::axes(Val::Px(10.0), Val::Px(3.0)), border: UiRect::all(Val::Px(2.0)), ..default() },
        BackgroundColor(BUTTON),
        BorderColor(BORDER),
        Clickable { act, left, order: order.0 },
    ));
    if let Some(cell) = cell {
        button.insert(GridPos(cell));
    }
    button.with_children(|button| label(button, text, 13.0, color));
}

/// A greyed-out row that cannot be focused.
pub fn disabled(parent: &mut ChildSpawnerCommands, text: impl Into<String>, width: Val) {
    parent
        .spawn((Node { width, padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)), border: UiRect::all(Val::Px(2.0)), ..default() }, BorderColor(Color::srgba(0.3, 0.4, 0.42, 0.35))))
        .with_children(|row| label(row, text, 17.0, DIM));
}

/// The button whose centre lies furthest along `dir` from `from` with the least sideways
/// offset: what a d-pad press in that direction should move to.
fn neighbour(from: Vec2, dir: Vec2, others: impl Iterator<Item = (usize, Vec2)>) -> Option<usize> {
    others
        .filter_map(|(rank, at)| {
            let delta = at - from;
            let along = delta.dot(dir);
            (along > 4.0).then(|| (rank, along + delta.perp_dot(dir).abs() * 2.5))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(rank, _)| rank)
}

/// Focus, hover, click and gamepad / keyboard activation for every button on screen.
///
/// The d-pad, stick and arrow keys move focus to the nearest button in that direction,
/// so rows of buttons work left to right and columns up and down. Value buttons
/// (steppers) use left / right to change their value instead.
#[allow(clippy::too_many_arguments)]
fn interact(
    controls: Res<Controls>,
    mut focus: ResMut<Focus>,
    mut buttons: Query<(&Interaction, &Clickable, &mut BackgroundColor, &mut BorderColor, &GlobalTransform, Option<&GridPos>)>,
    changed: Query<(&Interaction, &Clickable), Changed<Interaction>>,
    mut acts: EventWriter<ActEvent>,
    mut sounds: EventWriter<PlaySfx>,
    mut focused: ResMut<Focused>,
    mut request: ResMut<FocusRequest>,
    mut editing: ResMut<Editing>,
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
    let mut before = focus.0.min(orders.len() - 1);
    if let Some(wanted) = request.0.clone()
        && let Some((_, clickable, ..)) = buttons.iter().find(|(_, clickable, ..)| clickable.act == wanted)
    {
        before = rank(clickable.order);
        request.0 = None;
    }
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
    let current_order = orders[now];
    let focused_button = buttons.iter().find(|(_, clickable, ..)| clickable.order == current_order);
    let stepper = focused_button.is_some_and(|(_, clickable, ..)| clickable.left.is_some());
    // Editing a value: A started it, left / right change it, A or B (or moving off) ends it.
    if editing.0.is_some_and(|order| order != current_order) {
        editing.0 = None;
    }
    let mut swallow_back = false;
    if let Some(order) = editing.0 {
        if nav.accept || nav.back || nav.up || nav.down {
            editing.0 = None;
            swallow_back = nav.back;
            sounds.write(PlaySfx::ui(Sfx::UiMove));
        } else if let Some((_, clickable, ..)) = buttons.iter().find(|(_, c, ..)| c.order == order) {
            if nav.right {
                acts.write(ActEvent(clickable.act.clone()));
                fired = true;
            } else if nav.left
                && let Some(left) = &clickable.left
            {
                acts.write(ActEvent(left.clone()));
                fired = true;
            }
        }
    } else {
        // Moving: along a row / between rows of a grid when the buttons are laid out on one,
        // otherwise toward the nearest button on screen in that direction.
        let cells: Vec<(usize, IVec2)> = buttons.iter().filter_map(|(_, clickable, _, _, _, cell)| cell.map(|c| (rank(clickable.order), c.0))).collect();
        let here_cell = cells.iter().find(|(r, _)| *r == now).map(|(_, c)| *c);
        let step = |dir: IVec2| -> Option<usize> {
            let here = here_cell?;
            cells
                .iter()
                .filter(|(r, c)| *r != now && (*c - here).dot(dir) > 0)
                .min_by_key(|(_, c)| {
                    let d = *c - here;
                    let along = d.dot(dir).abs();
                    let across = (d - dir * d.dot(dir)).abs();
                    (along, across.x + across.y)
                })
                .map(|(r, _)| *r)
        };
        let centres: Vec<(usize, Vec2)> = buttons.iter().map(|(_, clickable, _, _, at, _)| (rank(clickable.order), at.translation().truncate())).collect();
        let here = centres.iter().find(|(r, _)| *r == now).map_or(Vec2::ZERO, |(_, at)| *at);
        let others = || centres.iter().copied().filter(|(r, _)| *r != now);
        if nav.down {
            now = step(IVec2::Y).or_else(|| neighbour(here, Vec2::Y, others())).unwrap_or((now + 1) % orders.len());
        } else if nav.up {
            now = step(IVec2::NEG_Y).or_else(|| neighbour(here, Vec2::NEG_Y, others())).unwrap_or((now + orders.len() - 1) % orders.len());
        } else if nav.left || nav.right {
            let (dir, flat) = if nav.right { (IVec2::X, Vec2::X) } else { (IVec2::NEG_X, Vec2::NEG_X) };
            if let Some(next) = step(dir).or_else(|| neighbour(here, flat, others())) {
                now = next;
            }
        } else if nav.accept
            && let Some((_, clickable, ..)) = focused_button
        {
            if stepper {
                // A value: start editing it.
                editing.0 = Some(clickable.order);
                sounds.write(PlaySfx::ui(Sfx::UiAccept));
            } else {
                acts.write(ActEvent(clickable.act.clone()));
                fired = true;
            }
        }
    }
    if nav.back && !swallow_back {
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

    for (_, clickable, mut background, mut border, _, _) in &mut buttons {
        let focused = clickable.order == orders[now];
        let (fill, edge) = if editing.0 == Some(clickable.order) {
            (BUTTON_EDIT, GOOD)
        } else if focused {
            (BUTTON_FOCUS, BORDER_FOCUS)
        } else {
            (BUTTON, BORDER)
        };
        if background.0 != fill {
            background.0 = fill;
            border.0 = edge;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::UiNav;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_event::<ActEvent>()
            .add_event::<PlaySfx>()
            .init_resource::<Controls>()
            .init_resource::<Focus>()
            .init_resource::<Focused>()
            .init_resource::<FocusRequest>()
            .init_resource::<Editing>()
            .add_systems(Update, interact);
        app
    }

    fn press(app: &mut App, nav: UiNav) -> Vec<Act> {
        app.world_mut().resource_mut::<Controls>().ui = nav;
        app.update();
        app.world_mut().resource_mut::<Controls>().ui = UiNav::default();
        app.world_mut().resource_mut::<Events<ActEvent>>().drain().map(|e| e.0).collect()
    }

    #[test]
    fn the_dpad_walks_the_grid_and_values_edit_on_a() {
        let mut app = app();
        // Row 0: two buttons. Row 1: a value (team size) under the first.
        let mut spawn = |order: u32, act: Act, left: Option<Act>, cell: (i32, i32)| {
            app.world_mut().spawn((Interaction::None, Clickable { act, left, order }, BackgroundColor(BUTTON), BorderColor(BORDER), GlobalTransform::default(), GridPos(IVec2::new(cell.0, cell.1))));
        };
        spawn(1, Act::Solo, None, (0, 0));
        spawn(2, Act::Host, None, (1, 0));
        spawn(3, Act::TeamSize(1), Some(Act::TeamSize(-1)), (0, 1));
        app.update();
        let focused = |app: &App| app.world().resource::<Focused>().0.clone();
        assert_eq!(focused(&app), Some(Act::Solo));
        press(&mut app, UiNav { right: true, ..default() });
        assert_eq!(focused(&app), Some(Act::Host));
        press(&mut app, UiNav { left: true, ..default() });
        assert_eq!(focused(&app), Some(Act::Solo));
        press(&mut app, UiNav { down: true, ..default() });
        assert_eq!(focused(&app), Some(Act::TeamSize(1)));
        // On a value, A starts editing (and changes nothing by itself)...
        assert!(press(&mut app, UiNav { accept: true, ..default() }).is_empty());
        assert_eq!(press(&mut app, UiNav { right: true, ..default() }), vec![Act::TeamSize(1)]);
        assert_eq!(press(&mut app, UiNav { left: true, ..default() }), vec![Act::TeamSize(-1)]);
        // ...B ends editing without leaving the screen, and then right just moves on.
        assert!(press(&mut app, UiNav { back: true, ..default() }).is_empty());
        assert!(press(&mut app, UiNav { right: true, ..default() }).is_empty());
        assert_eq!(focused(&app), Some(Act::Host));
        press(&mut app, UiNav { down: true, ..default() });
        press(&mut app, UiNav { up: true, ..default() });
        assert_eq!(focused(&app), Some(Act::Solo));
    }
}
