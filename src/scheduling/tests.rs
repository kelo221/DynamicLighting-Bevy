use std::thread::ThreadId;

use bevy::app::SubApp;
use bevy::core_pipeline::Core3d;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::{ExtractSchedule, Render, RenderScheduleOrder};

use super::SingleThreadedSchedulesPlugin;
use super::executors::{single_threaded, switch_render};

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

#[test]
fn main_schedules_run_on_the_thread_that_updates_the_app() {
    let here = std::thread::current().id();
    let threads = update_threads(Some(SingleThreadedSchedulesPlugin::default()));
    assert_eq!(threads, vec![here; 3]);
    // The multi-threaded executor runs them on its workers, so the check above means something.
    let threads = update_threads(None);
    assert_eq!(threads.len(), 3);
    assert!(threads.iter().all(|&thread| thread != here));
    let main_off = SingleThreadedSchedulesPlugin {
        main: false,
        ..default()
    };
    assert!(
        update_threads(Some(main_off))
            .iter()
            .all(|&thread| thread != here)
    );
}

#[test]
fn extraction_still_leaves_its_commands_to_the_render_world() {
    let mut render_app = SubApp::new();
    render_app.init_resource::<RenderScheduleOrder>();
    render_app.init_resource::<Threads>();
    let mut extract = Schedule::new(ExtractSchedule);
    extract.add_systems(spawn).set_apply_final_deferred(false);
    render_app.add_schedule(extract);
    render_app.add_systems(Render, note_thread);
    let render_switched = !cfg!(any(target_os = "macos", target_os = "ios"));
    // Every sub-app has a `First`.
    let expected = 2 + usize::from(render_switched);
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

    world.run_schedule(Render);
    let ran_here = world.resource::<Threads>().0 == vec![std::thread::current().id()];
    assert_eq!(ran_here, render_switched);
    assert!(
        render_app.get_schedule(Core3d).is_none(),
        "a schedule the app doesn't have isn't created"
    );
}
