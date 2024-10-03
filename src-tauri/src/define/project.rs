use log::info;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufReader, BufWriter, ErrorKind, Write},
    mem::size_of,
    path::PathBuf,
    vec,
};
use tauri::api::dialog::blocking::FileDialogBuilder;

use crate::{
    define::serialize::{make_fnis_lines, map_race_to_folder},
    racekeys::map_legacy_to_racekey,
};

use super::{
    position::Sex,
    scene::{Node, Scene},
    serialize::EncodeBinary,
    stage::Stage,
    NanoID, NANOID_ALPHABET, PREFIX_HASH_LEN,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Project {
    #[serde(skip)]
    pub pack_path: PathBuf,

    pub pack_name: String,
    pub pack_author: String,
    pub prefix_hash: String,
    pub scenes: HashMap<NanoID, Scene>,
}

impl Project {
    pub fn new() -> Self {
        Self {
            pack_path: Default::default(),

            pack_name: Default::default(),
            pack_author: "Unknown".into(),
            prefix_hash: nanoid!(PREFIX_HASH_LEN, &NANOID_ALPHABET),
            scenes: HashMap::new(),
        }
    }

    pub fn reset(&mut self) -> &Self {
        *self = Self::new();

        self
    }

    pub fn save_scene(&mut self, scene: Scene) -> &Scene {
        let id = scene.id.clone();
        info!("Saving or inserting Scene: {} / {}", id, scene.name);
        self.scenes.insert(id.clone(), scene);
        self.scenes.get(&id).unwrap()
    }

    pub fn discard_scene(&mut self, id: &NanoID) -> Option<Scene> {
        let ret = self.scenes.remove(id);
        info!(
            "Deleting Scene: {} / {}",
            id,
            ret.as_ref()
                .and_then(|s| Some(s.name.as_str()))
                .unwrap_or_default()
        );
        ret
    }

    pub fn get_scene(&self, id: &NanoID) -> Option<&Scene> {
        self.scenes.get(id)
    }

    pub fn get_scene_mut(&mut self, id: &NanoID) -> Option<&mut Scene> {
        self.scenes.get_mut(id)
    }

    pub fn get_stage(&self, id: &NanoID) -> Option<&Stage> {
        for (_, scene) in &self.scenes {
            let stage = scene.get_stage(id);
            if stage.is_some() {
                return stage;
            }
        }
        None
    }

    pub fn load_project(&mut self) -> Result<(), String> {
        let path = FileDialogBuilder::new()
            .add_filter("SL Project File", vec!["slsb.json"].as_slice())
            .pick_file()
            .ok_or("No path to load project from".to_string())?;
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let value = Project::from_file(file)?;

        *self = value;
        self.set_project_name_from_path(&path);
        self.pack_path = path;

        Ok(())
    }

    pub fn from_file(file: std::fs::File) -> Result<Project, String> {
        let project: Project =
            serde_json::from_reader(BufReader::new(file)).map_err(|e| e.to_string())?;
        println!("Loaded project {}", project.pack_name);
        Ok(project)
    }

    pub fn save_project(&mut self, save_as: bool) -> Result<(), String> {
        let path = if save_as || !self.pack_path.exists() || self.pack_path.is_dir() {
            let f = FileDialogBuilder::new()
                .set_file_name(&self.pack_name)
                .add_filter("SL Project File", vec!["slsb.json"].as_slice())
                .save_file();
            if f.is_none() {
                return Err("No path to save project to".into());
            }
            f.unwrap()
        } else {
            self.pack_path.clone()
        };

        self.set_project_name_from_path(&path);

        self.write(path)
    }

    pub fn write(&mut self, path: PathBuf) -> Result<(), String> {
        let file = fs::File::create(&path).map_err(|e| e.to_string())?;
        serde_json::to_writer(file, self).map_err(|e| e.to_string())?;
        println!("Saved project {}", self.pack_name);
        Ok(())
    }

    pub fn load_slal(&mut self) -> Result<(), String> {
        let path = FileDialogBuilder::new()
            .add_filter("SLAL File", vec!["json"].as_slice())
            .pick_file();
        if path.is_none() {
            return Err("No path to load slal file from".into());
        }

        let path = path.unwrap();

        match Project::from_slal(path) {
            Ok(prjct) => {
                *self = prjct;
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    pub fn from_slal(path: PathBuf) -> Result<Project, String> {
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;

        let slal: serde_json::Value =
            serde_json::from_reader(BufReader::new(file)).map_err(|e| e.to_string())?;

        let mut prjct = Project::new();
        prjct.pack_name = slal["name"]
            .as_str()
            .ok_or("Missing name attribute")?
            .into();

        let anims = slal["animations"]
            .as_array()
            .ok_or("Missing animations attribute")?;
        for animation in anims {
            let mut scene = Scene::default();
            scene.name = animation["name"]
                .as_str()
                .ok_or("Missing name attribute")?
                .into();
            let crt_race = animation["creature_race"].as_str().unwrap_or_default();
            let actors = animation["actors"]
                .as_array()
                .ok_or("Missing actors attribute")?;

            // initialize stages and copy information for every position into the respective stage
            for (n, position) in actors.iter().enumerate() {
                let sex = position["type"].as_str().unwrap_or("male").to_lowercase();
                let events = position["stages"]
                    .as_array()
                    .ok_or("Missing stages attribute")?;

                if scene.stages.is_empty() {
                    for _ in 0..events.len() {
                        scene.stages.push(Default::default());
                    }
                    if scene.stages.is_empty() {
                        return Err("Scene has no stages".into());
                    }
                    for stage in &mut scene.stages {
                        stage.positions = vec![Default::default(); actors.len()];
                    }
                }
                for (i, evt) in events.iter().enumerate() {
                    let edit_position = &mut scene.stages[i].positions[n];
                    edit_position.event =
                        vec![evt["id"].as_str().ok_or("Missing id attribute")?.into()];
                    match sex.as_str() {
                        "male" | "type" => {
                            edit_position.sex = Sex {
                                male: true,
                                female: false,
                                futa: false,
                            };
                            edit_position.race = "Human".into();
                        }
                        "female" => {
                            edit_position.sex = Sex {
                                male: false,
                                female: true,
                                futa: false,
                            };
                            edit_position.race = "Human".into();
                        }
                        "creaturemale" => {
                            edit_position.sex = Sex {
                                male: true,
                                female: false,
                                futa: false,
                            };
                            edit_position.race = map_legacy_to_racekey(
                                position["race"].as_str().unwrap_or(crt_race),
                            )?;
                        }
                        "creaturefemale" => {
                            edit_position.sex = Sex {
                                male: false,
                                female: true,
                                futa: false,
                            };
                            edit_position.race = map_legacy_to_racekey(
                                position["race"].as_str().unwrap_or(crt_race),
                            )?;
                        }
                        _ => {
                            return Err(format!("Unrecognized gender: {}", sex));
                        }
                    }
                }
            }
            // finalize stage data, adding climax to last positions
            let tags = animation["tags"]
                .as_str()
                .and_then(|tags| {
                    let list = tags
                        .to_lowercase()
                        .split(',')
                        .map(|str| str.trim().to_string())
                        .collect::<Vec<_>>();
                    Some(list)
                })
                .unwrap_or_default();
            let stage_extra = animation["stage"].as_array();
            for (i, stage) in scene.stages.iter_mut().enumerate() {
                stage.tags = tags.clone();
                if let Some(extra_vec) = stage_extra {
                    for extra in extra_vec {
                        let n = extra["number"].as_i64().unwrap_or(-1);
                        if n == -1 || n as usize != i {
                            continue;
                        }
                        stage.extra.fixed_len = extra["timer"].as_f64().unwrap_or_default() as f32;
                    }
                }
            }
            let last = scene.stages.last_mut().unwrap();
            for position in &mut last.positions {
                position.extra.climax = true;
            }
            // build graph
            scene.root = scene.stages[0].id.clone();
            let mut prev_id: Option<String> = None;
            for stage in scene.stages.iter_mut().rev() {
                let mut value = Node::default();
                if let Some(id) = prev_id {
                    value.dest = vec![id];
                }
                scene.graph.insert(stage.id.clone(), value);
                prev_id = Some(stage.id.clone());
            }
            // add to prjct
            prjct.scenes.insert(scene.id.clone(), scene);
        }

        println!(
            "Loaded {} Animations from {}",
            prjct.scenes.len(),
            path.to_str().unwrap_or_default()
        );

        Ok(prjct)
    }

    pub fn export(&self) -> Result<(), std::io::Error> {
        let path = FileDialogBuilder::new().pick_folder();
        if path.is_none() {
            return Err(std::io::Error::from(ErrorKind::Interrupted));
        }
        let root_dir = path.unwrap();
        self.build(root_dir)
    }

    pub fn build(&self, root_dir: PathBuf) -> Result<(), std::io::Error> {
        println!("Compiling project {}", self.pack_name);
        // Write binary
        {
            let target_dir = root_dir.join("SKSE\\SexLab\\Registry\\");
            let mut buf: Vec<u8> = Vec::new();
            buf.reserve(self.get_byte_size());
            self.write_byte(&mut buf);
            fs::create_dir_all(&target_dir)?;
            let mut file = fs::File::create(target_dir.join(format!(
                "{}.slr",
                if self.pack_name.is_empty() {
                    &self.prefix_hash
                } else {
                    &self.pack_name
                }
            )))?;
            file.write_all(&buf)?;
        }
        // Write FNIS files
        {
            let mut events: HashMap<&str, Vec<String>> = HashMap::new(); // map<RaceKey, Lines[]>
            let mut control: HashSet<&str> = HashSet::from(["__BLANK__", "__DEFAULT__"]);
            for (_, scene) in &self.scenes {
                if scene.has_warnings {
                    continue;
                }
                for stage in &scene.stages {
                    for position in &stage.positions {
                        let event = &position.event[0];
                        if control.contains(event.as_str()) {
                            continue;
                        }
                        control.insert(event);
                        
                        let lines = if (std::env::var("UD_WORKAROUND")).is_ok() { 
                            [make_fnis_lines(
                                &position.event,
                                "",
                                stage.extra.fixed_len > 0.0,
                                &position.anim_obj.split(',').fold(vec![], |mut acc, x| {
                                    if !x.is_empty() {
                                        acc.push(x.to_string());
                                    }
                                    acc
                                }),
                            ),
                            make_fnis_lines(
                                &position.event,
                                &self.prefix_hash,
                                stage.extra.fixed_len > 0.0,
                                &position.anim_obj.split(',').fold(vec![], |mut acc, x| {
                                    if !x.is_empty() {
                                        acc.push(x.to_string());
                                    }
                                    acc
                                }),
                            )].concat()
                        } else {
                            make_fnis_lines(
                                &position.event,
                                &self.prefix_hash,
                                stage.extra.fixed_len > 0.0,
                                &position.anim_obj.split(',').fold(vec![], |mut acc, x| {
                                    if !x.is_empty() {
                                        acc.push(x.to_string());
                                    }
                                    acc
                                }),
                            )
                        };
                        
                        let mut insert = |race| {
                            events
                                .entry(race)
                                .and_modify(|list| list.append(&mut lines.clone()))
                                .or_insert(lines.clone());
                        };
                        let race = position.race.as_str();
                        match race {
                            "Canine" => {
                                insert(&position.race);
                                insert("Dog");
                                insert("Wolf");
                            }
                            "Dog" | "Wolf" => {
                                insert(&position.race);
                                insert("Canine");
                            }
                            //  => {
                            //     insert("Boar");
                            //     insert("Boar (Mounted)");
                            // }
                            "Boar" | "Boar (Mounted)" | "Boar (Any)" => insert("Boar (Any)"),
                            _ => insert(&position.race),
                        }
                    }
                }
            }
            info!("---------------------------------------------------------");
            for (racekey, anim_events) in events {
                let target_folder = map_race_to_folder(racekey)
                    .expect(format!("Cannot find folder for RaceKey {}", racekey).as_str());
                let path = root_dir.join(format!(
                    "meshes\\actors\\{}\\animations\\{}",
                    target_folder, self.pack_name
                ));
                let crt = &target_folder[target_folder
                    .find('\\')
                    .and_then(|w| Some(w + 1))
                    .unwrap_or(0)..];
                fs::create_dir_all(&path)?;

                let create = |file_path: PathBuf| -> Result<(), std::io::Error> {
                    let name = file_path.to_str().unwrap_or("NONE".into()).to_string();
                    let file = fs::File::create(file_path)?;
                    let mut file = BufWriter::new(file);
                    info!(
                        "Adding {} lines to race {} |||||| file: {}",
                        anim_events.len(),
                        racekey,
                        name
                    );
                    for anim_event in anim_events {
                        writeln!(file, "{}", anim_event)?;
                    }
                    Ok(())
                };
                match crt {
                    "character" => create(path.join(format!("FNIS_{}_List.txt", self.pack_name))),
                    "canine" => match racekey {
                        "Canine" => {
                            create(path.join(format!("FNIS_{}_canine_List.txt", self.pack_name)))
                        }
                        "Dog" => create(path.join(format!("FNIS_{}_dog_List.txt", self.pack_name))),
                        _ => create(path.join(format!("FNIS_{}_wolf_List.txt", self.pack_name))),
                    },
                    _ => create(path.join(format!("FNIS_{}_{}_List.txt", self.pack_name, crt))),
                }?;
            }
        }
        info!(
            "Successfully compiled {}",
            root_dir.to_str().unwrap_or_default()
        );
        Ok(())
    }

    pub fn import_offset(&mut self) -> Result<(), String> {
        let path = FileDialogBuilder::new()
            .add_filter("Offset File", vec!["yaml"].as_slice())
            .pick_file()
            .ok_or("No path to load offsets from".to_string())?;
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let offsetfile: serde_yaml::Mapping =
            serde_yaml::from_reader(BufReader::new(file)).map_err(|e| e.to_string())?;

        for (scene_id_v, stages_v) in offsetfile {
            if !stages_v.is_mapping() {
                continue;
            }
            let scene_id = scene_id_v
                .as_str()
                .ok_or("Not a valid offset file, expected string for scene id".to_string())?
                .to_string();
            if let Some(scene) = self.get_scene_mut(&scene_id) {
                scene.import_offset(
                    stages_v
                        .as_mapping()
                        .ok_or(format!("Expected mapping in scene {}", scene_id))?,
                )?;
            }
        }

        Ok(())
    }

    fn set_project_name_from_path(&mut self, path: &PathBuf) -> () {
        self.pack_name = String::from(
            path.file_name() // ...\\{project.slsb.json}
                .and_then(|name| name.to_str())
                .and_then(|str| {
                    let ret = &str[0..str.find(".slsb.json").unwrap_or(str.len())];
                    Some(ret)
                })
                .unwrap_or_default(),
        );
    }
}

impl EncodeBinary for Project {
    fn get_byte_size(&self) -> usize {
        let mut ret = self.pack_author.len()
            + self.pack_name.len()
            + 3 * size_of::<u64>()
            + PREFIX_HASH_LEN
            + 1;
        for (_, value) in &self.scenes {
            if value.has_warnings {
                continue;
            }
            ret += value.get_byte_size();
        }

        ret
    }

    fn write_byte(&self, buf: &mut Vec<u8>) -> () {
        // version
        let version: u8 = 3;
        buf.push(version);
        // project
        buf.extend_from_slice(&(self.pack_name.len() as u64).to_be_bytes());
        buf.extend_from_slice(self.pack_name.as_bytes());
        buf.extend_from_slice(&(self.pack_author.len() as u64).to_be_bytes());
        buf.extend_from_slice(self.pack_author.as_bytes());
        buf.extend_from_slice(self.prefix_hash.as_bytes());
        buf.extend_from_slice(&(self.scenes.len() as u64).to_be_bytes());
        for (_, scene) in &self.scenes {
            if scene.has_warnings {
                continue;
            }
            if scene.stages.len() == 0 {
                panic!("Empty Scene whilst building files");
            }
            scene.write_byte(buf);
        }
    }
}
