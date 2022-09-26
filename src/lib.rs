use std::cell::RefCell;
use std::collections::HashMap;

use log::*;
use screeps::{
    find, game, prelude::*, Creep, ObjectId, Part, ResourceType, ReturnCode, RoomObjectProperties,
    Source, StructureController, StructureObject,
};
use wasm_bindgen::prelude::*;

mod logging;

// add wasm_bindgen to any function you would like to expose for call from js
#[wasm_bindgen]
pub fn setup() {
    logging::setup_logging(logging::Info);
}

// this enum will represent a creep's lock on a specific target object, storing a js reference to the object id so that we can grab a fresh reference to the object each successive tick, since screeps game objects become 'stale' and shouldn't be used beyond the tick they were fetched
#[derive(Clone, Copy)]
enum CreepTarget {
    Upgrade(ObjectId<StructureController>),
    Harvest(ObjectId<Source>),
}

struct CreepState {
    target: Option<CreepTarget>,
}

// this is one way to persist data between ticks within Rust's memory, as opposed to
// keeping state in memory on game objects - but will be lost on global resets!
thread_local! {
    static CREEP_STATES: RefCell<HashMap<String, CreepState>> = RefCell::new(HashMap::new());
}

// to use a reserved name as a function name, use `js_name`:
#[wasm_bindgen(js_name = loop)]
pub fn game_loop() {
    debug!("loop starting! CPU: {}", game::cpu::get_used());
    run_creeps();
    debug!("running spawns");
    spawn_creeps();
    info!("done! cpu: {}", game::cpu::get_used())
}

fn run_creeps() {
    // mutably borrow the creep_targets refcell, which is holding our creep target locks
    // in the wasm heap
    CREEP_STATES.with(|creep_states_refcell| {
        let mut creep_states = creep_states_refcell.borrow_mut();
        debug!("running creeps");
        // same type conversion (and type assumption) as the spawn loop
        for creep in game::creeps().values() {
            let creep_name = creep.name();
            debug!("running creep {}", creep_name);

            match creep_states.remove(&creep_name) {
                Some(creep_state) => {
                    let creep_state = run_creep(&creep, creep_state);
                    creep_states.insert(creep_name, creep_state);
                }
                None => {
                    creep_states.insert(creep_name, CreepState { target: None });
                }
            }
        }
    });
}

fn run_creep(creep: &Creep, creep_state: CreepState) -> CreepState {
    if creep.spawning() {
        return creep_state;
    }

    return match creep_state.target {
        Some(creep_target) => {
            let keep_target = run_creep_by_target(creep, &creep_target);

            CreepState {
                target: if keep_target {
                    creep_state.target
                } else {
                    find_target(creep)
                },
            }
        }
        None => CreepState {
            target: find_target(creep),
        },
    };
}

fn run_creep_by_target(creep: &Creep, creep_target: &CreepTarget) -> bool {
    return match &creep_target {
        CreepTarget::Upgrade(controller_id) => run_creep_upgrade(creep, controller_id),
        CreepTarget::Harvest(source_id) => run_creep_harvest(creep, source_id),
    };
}

fn run_creep_upgrade(creep: &Creep, controller_id: &ObjectId<StructureController>) -> bool {
    if creep.store().get_used_capacity(Some(ResourceType::Energy)) <= 0 {
        return false;
    }

    return match controller_id.resolve() {
        Some(controller) => match creep.upgrade_controller(&controller) {
            ReturnCode::Ok => true,
            ReturnCode::NotInRange => {
                creep.move_to(&controller);
                true
            }
            _ => false,
        },
        None => false,
    };
}

fn run_creep_harvest(creep: &Creep, source_id: &ObjectId<Source>) -> bool {
    if creep.store().get_free_capacity(Some(ResourceType::Energy)) <= 0 {
        return false;
    }

    return match source_id.resolve() {
        Some(source) => match creep.harvest(&source) {
            ReturnCode::Ok => true,
            ReturnCode::NotInRange => {
                creep.move_to(&source);
                true
            }
            _ => false,
        },
        None => false,
    };
}

fn find_target(creep: &Creep) -> Option<CreepTarget> {
    let room = creep.room().expect("couldn't resolve creep room");

    if creep.store().get_used_capacity(Some(ResourceType::Energy)) > 0 {
        for structure in room.find(find::STRUCTURES).iter() {
            // find a structure and upgrade it
            if let StructureObject::StructureController(controller) = structure {
                return Some(CreepTarget::Upgrade(controller.id()));
            }
        }
    }

    return match creep.pos().find_closest_by_path(find::SOURCES_ACTIVE, None) {
        Some(source) => Some(CreepTarget::Harvest(source.id())),
        None => None,
    };
}

fn spawn_creeps() {
    // Game::spawns returns a `js_sys::Object`, which is a light reference to an
    // object of any kind which is held on the javascript heap.
    //
    // Object::values returns a `js_sys::Array`, which contains the member spawn objects
    // representing all the spawns you control.
    //
    // They are returned as wasm_bindgen::JsValue references, which we can safely
    // assume are StructureSpawn objects as returned from js without checking first
    let mut additional = 0;
    for spawn in game::spawns().values() {
        debug!("running spawn {}", String::from(spawn.name()));

        let body = [Part::Move, Part::Move, Part::Carry, Part::Work];
        if spawn.room().unwrap().energy_available() >= body.iter().map(|p| p.cost()).sum() {
            // create a unique name, spawn.
            let name_base = game::time();
            let name = format!("{}-{}", name_base, additional);
            // note that this bot has a fatal flaw; spawning a creep
            // creates Memory.creeps[creep_name] which will build up forever;
            // these memory entries should be prevented (todo doc link on how) or cleaned up
            let res = spawn.spawn_creep(&body, &name);

            // todo once fixed in branch this should be ReturnCode::Ok instead of this i8 grumble grumble
            if res != ReturnCode::Ok {
                warn!("couldn't spawn: {:?}", res);
            } else {
                additional += 1;
            }
        }
    }
}
