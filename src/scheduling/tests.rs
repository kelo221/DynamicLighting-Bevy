use std::thread::ThreadId;

use bevy::app::{FixedUpdate, MainScheduleOrder, SubApp};
use bevy::core_pipeline::Core3d;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::{ExtractSchedule, Render, RenderApp, RenderScheduleOrder};

use super::executors::{single_threaded, switch_main, switch_render};
use super::{MainSchedules, SingleThreadedSchedulesPlugin};

#[derive(Component)]
struct Spawned;

fn spawn(mut commands: Commands) {
    commands.spawn(Spawned);
}

fn spawned(world: &mut World) -> usize {
    world.query::<&Spawned>().iter(world).count()
}

/// The threads that ran [`note_thread`].
#[derive(Resource, Default)]
struct Threads(Vec<ThreadId>);

fn note_thread(mut threads: ResMut<Threads>) {
    threads.0.push(std::thread::current().id());
}

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct Test;

/// Bevy picks the single-threaded executor for every schedule on the web.
const MULTI_THREADED: bool = !cfg!(target_arch = "wasm32");
/// Where `Render` keeps its executor.
const RENDER_SWITCHES: bool = !cfg!(any(target_os = "macos", target_os = "ios"));

#[test]
fn switched_schedules_apply_commands_at_their_end_only_where_they_did() {
    for apply in [true, false] {
        let mut world = World::new();
        let mut schedule = Schedule::new(Test);
        schedule.add_systems(spawn);
        single_threaded(&mut schedule, apply);
        schedule.run(&mut world);
        assert_eq!(spawned(&mut world), usize::from(apply));
        schedule.apply_deferred(&mut world);
        assert_eq!(spawned(&mut world), 1);
    }
}

/// The threads that ran `Update`, `PostUpdate` and `Last` in one update of an app.
fn update_threads(plugin: Option<SingleThreadedSchedulesPlugin>) -> Vec<ThreadId> {
    let mut app = App::new();
    app.init_resource::<Threads>()
        .add_systems(Update, note_thread)
        .add_systems(PostUpdate, note_thread)
        .add_systems(Last, note_thread);
    if let Some(plugin) = plugin {
        app.add_plugins(plugin);
    }
    app.finish();
    app.cleanup();
    app.update();
    app.world_mut().remove_resource::<Threads>().unwrap().0
}

fn with_main(main: MainSchedules) -> Option<SingleThreadedSchedulesPlugin> {
    Some(SingleThreadedSchedulesPlugin { main, ..default() })
}

/// Which of `Update`, `PostUpdate` and `Last` ran on the thread that updated the app.
fn ran_here(plugin: Option<SingleThreadedSchedulesPlugin>) -> Vec<bool> {
    let here = std::thread::current().id();
    let threads = update_threads(plugin);
    assert_eq!(threads.len(), 3);
    threads.iter().map(|&thread| thread == here).collect()
}

#[test]
fn main_schedules_run_on_the_thread_that_updates_the_app() {
    assert_eq!(ran_here(with_main(MainSchedules::All)), [true; 3]);
    // The multi-threaded executor runs them on its workers, so the checks mean something.
    if MULTI_THREADED {
        assert_eq!(ran_here(None), [false; 3]);
        assert_eq!(ran_here(with_main(MainSchedules::Off)), [false; 3]);
    }
}

/// By default `Update` keeps the multi-threaded executor and the engine's schedules switch.
#[test]
fn the_default_leaves_update_alone() {
    assert_eq!(
        SingleThreadedSchedulesPlugin::default().main,
        MainSchedules::Engine
    );
    let expected = [!MULTI_THREADED, true, true];
    assert_eq!(ran_here(Some(default())), expected);
    assert_eq!(ran_here(with_main(MainSchedules::Engine)), expected);
}

/// A schedule an app adds to the main schedule order keeps its executor and its settings,
/// here not applying its commands at its end; Bevy's fixed-timestep schedules switch.
#[test]
fn only_bevys_own_main_schedules_switch() {
    let mut app = SubApp::new();
    app.init_resource::<MainScheduleOrder>();
    let mut own = Schedule::new(Test);
    own.add_systems(spawn).set_apply_final_deferred(false);
    app.add_schedule(own);
    app.world_mut()
        .resource_mut::<MainScheduleOrder>()
        .insert_after(Update, Test);
    app.add_systems(FixedUpdate, spawn);
    // Every sub-app has a `First`.
    assert_eq!(switch_main(&mut app, MainSchedules::Off), 0);
    assert_eq!(switch_main(&mut app, MainSchedules::Engine), 1, "First");
    assert_eq!(
        switch_main(&mut app, MainSchedules::All),
        2,
        "First and FixedUpdate"
    );

    let world = app.world_mut();
    world.run_schedule(Test);
    assert_eq!(spawned(world), 0, "left for the app to apply");
    world.run_schedule(FixedUpdate);
    assert_eq!(spawned(world), 1);
}

#[test]
fn extraction_still_leaves_its_commands_to_the_render_world() {
    let mut render_app = SubApp::new();
    render_app.init_resource::<RenderScheduleOrder>();
    let mut extract = Schedule::new(ExtractSchedule);
    extract.add_systems(spawn).set_apply_final_deferred(false);
    render_app.add_schedule(extract);
    render_app.add_systems(Render, || {});
    let expected = 2 + usize::from(RENDER_SWITCHES);
    assert_eq!(
        switch_render(&mut render_app),
        expected,
        "First, Render, extraction"
    );

    let world = render_app.world_mut();
    world.run_schedule(ExtractSchedule);
    assert_eq!(
        spawned(world),
        0,
        "applied by the render world's own system"
    );
    world.resource_scope(|world, mut schedules: Mut<Schedules>| {
        schedules
            .get_mut(ExtractSchedule)
            .unwrap()
            .apply_deferred(world);
    });
    assert_eq!(spawned(world), 1);
    assert!(
        render_app.get_schedule(Core3d).is_none(),
        "a schedule the app doesn't have isn't created"
    );
}

/// `Render` runs on the thread that runs the render world once switched, and `render: false`
/// leaves it on the multi-threaded executor's workers.
#[test]
fn render_switches_only_when_asked() {
    for render in [true, false] {
        let mut render_app = SubApp::new();
        render_app.init_resource::<RenderScheduleOrder>();
        render_app.init_resource::<Threads>();
        render_app.add_systems(Render, note_thread);
        let mut app = App::new();
        app.insert_sub_app(RenderApp, render_app);
        app.add_plugins(SingleThreadedSchedulesPlugin {
            main: MainSchedules::All,
            render,
        });
        app.finish();

        let world = app.sub_app_mut(RenderApp).world_mut();
        world.run_schedule(Render);
        let ran_here = world.resource::<Threads>().0 == vec![std::thread::current().id()];
        assert_eq!(ran_here, !MULTI_THREADED || (render && RENDER_SWITCHES));
    }
}
