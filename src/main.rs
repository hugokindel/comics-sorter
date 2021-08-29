use clap::{Arg, App};
use regex::{Regex, Captures};
use std::path::Path;
use walkdir::WalkDir;
use lazy_static::lazy_static;
use std::fs;
use std::ffi::OsStr;
use std::process::Command;

lazy_static! {
    static ref REGEX_ISSUES: Regex = Regex::new(r"(?x)(?P<name>.*) (?P<number>\d{3}).*\((?P<year>\d{4})\)").unwrap();
    static ref REGEX_VOLUMES: Regex = Regex::new(r"(?x)(?P<name>.*) v(?P<number>\d{2}).*\((?P<year>\d{4})\)").unwrap();
    static ref REGEXES: Vec<&'static Regex> = vec![&REGEX_ISSUES, &REGEX_VOLUMES];
    static ref WATERMARK_NAMES: [&'static str; 5] = ["zWater.jpg", "zzTLK.jpg", "zSoU-Nerd.jpg", "ZZZZZ.jpg", "zzoronewtag10.jpg"];
}

fn get_extension_from_filename(filename: &str) -> Option<&str> {
    Path::new(filename)
        .extension()
        .and_then(OsStr::to_str)
}

fn unpack_rar<P: AsRef<Path>>(file_path: P, extraction_path: P) -> std::io::Result<()> {
    Command::new("unrar")
        .arg("e")
        .arg(file_path.as_ref().to_str().unwrap())
        .arg(extraction_path.as_ref().to_str().unwrap())
        .output()?;

    Ok(())
}

fn unpack_zip<P: AsRef<Path>>(file_path: P, extraction_path: P) -> std::io::Result<()> {
    Command::new("7z")
        .arg("e")
        .arg(file_path.as_ref().to_str().unwrap())
        .arg(format!("-o{}", extraction_path.as_ref().to_str().unwrap()))
        .arg("*.jpg")
        .arg("-r")
        .output()?;

    Ok(())
}

fn pack_rar<P: AsRef<Path>>(file_path: P, extraction_path: P) -> std::io::Result<()> {
    let mut rar_command = Command::new("rar");
    rar_command.arg("a")
        .arg(file_path.as_ref().to_str().unwrap());

    for path in fs::read_dir(extraction_path).unwrap() {
        rar_command.arg(format!("{}", fs::canonicalize(path.unwrap().path()).unwrap().to_str().unwrap().to_string()));
    }

    fs::remove_file(&file_path.as_ref())?;

    rar_command.output()?;

    Ok(())
}

fn pack_zip<P: AsRef<Path>>(file_path: P, extraction_path: P) -> std::io::Result<()> {
    fs::remove_file(&file_path.as_ref())?;

    Command::new("7z")
        .arg("a")
        .arg(file_path.as_ref().to_str().unwrap())
        .arg(format!("{}/*", extraction_path.as_ref().to_str().unwrap()))
        .output()?;

    Ok(())
}

fn handle_watermark<P: AsRef<Path>>(file_path: P, folder_path: P) -> std::io::Result<()> {
    let filename_withouth_extension = file_path.as_ref().file_stem().unwrap().to_str().unwrap();
    let extraction_path = folder_path.as_ref().join(filename_withouth_extension);

    fs::create_dir_all(&extraction_path)?;

    if file_path.as_ref().to_string_lossy().ends_with(".cbr") {
        unpack_rar(file_path.as_ref(), extraction_path.as_path())?
    } else if file_path.as_ref().to_string_lossy().ends_with(".cbz") {
        unpack_zip(file_path.as_ref(), extraction_path.as_path())?
    }

    for entry in WATERMARK_NAMES.iter() {
        let extraction_path = extraction_path.as_path();
        let watermark_path = extraction_path.join(entry);
        if watermark_path.exists() {
            fs::remove_file(watermark_path)?;

            let metadata_path = &extraction_path.join("ComicInfo.xml");

            if metadata_path.exists() {
                let metadata_str = fs::read_to_string(&metadata_path)?;
                fs::remove_file(&metadata_path)?;
                let re = Regex::new(r"<PageCount>(.*)</PageCount>").unwrap();
                let captured = re.captures(&*metadata_str);
                let result_str = format!("<PageCount>{}</PageCount>", &*(captured.unwrap()[1].parse::<i32>().unwrap() - 1).to_string());
                fs::write(metadata_path, &*re.replace_all(&*metadata_str, result_str))?;
            }

            if file_path.as_ref().to_string_lossy().ends_with(".cbr") {
                pack_rar(file_path.as_ref(), extraction_path)?;
            } else if file_path.as_ref().to_string_lossy().ends_with(".cbz") {
                pack_zip(file_path.as_ref(), extraction_path)?;
            }
        }
    }

    fs::remove_dir_all(extraction_path)?;

    Ok(())
}

fn sort_file<P: AsRef<Path>>(file_path: P, captures: Captures, output_path: P) -> std::io::Result<()> {
    let year = &captures["year"];
    let name = &captures["name"].trim();
    let number = &captures["number"].parse::<i32>().unwrap();
    let extension = get_extension_from_filename(file_path.as_ref().to_str().unwrap()).unwrap();
    let sorted_dir_path = output_path.as_ref().join(format!("{} - {}", year, name));

    fs::create_dir_all(&sorted_dir_path)?;
    let sorted_file_path = sorted_dir_path.join(format!("{:03}.{}", number, extension));
    fs::copy(file_path.as_ref(), sorted_file_path.as_path())?;

    handle_watermark(sorted_file_path.as_path(), sorted_dir_path.as_path())?;

    Ok(())
}

fn sort_folder<P: AsRef<Path>>(input_path: P, output_path: P) -> std::io::Result<()> {
    fs::create_dir_all(&output_path)?;

    let mut num_handled_files = 0;

    for entry in WalkDir::new(input_path.as_ref())
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| !e.file_type().is_dir()) {
        let abs_path = fs::canonicalize(entry.path()).unwrap();

        for regex in REGEXES.iter() {
            let filename = entry.path().file_name().unwrap();
            let captured = regex.captures(filename.to_str().unwrap());

            if captured.is_some() {
                num_handled_files += 1;
                sort_file(abs_path.as_path(), captured.unwrap(), output_path.as_ref())?;
                break;
            }
        }
    }

    if num_handled_files == 0 {
        fs::remove_dir(input_path.as_ref().join("output"))?;
    }

    Ok(())
}

fn main() -> std::io::Result<()> {
    let matches = App::new("comics_sort")
        .version("1.0")
        .author("KINDEL Hugo <me@hugokindel.xyz>")
        .about("Sort comics folders")
        .arg(Arg::with_name("INPUT")
            .help("Sets the input folder to sort")
            .required(true))
        .arg(Arg::with_name("OUTPUT")
            .short("o")
            .long("output")
            .help("Sets the output folder to use")
            .takes_value(true))
        .get_matches();

    let input_path_temp = fs::canonicalize(matches.value_of("INPUT").unwrap()).unwrap();
    let input_path = input_path_temp.as_path();
    let output_path = if matches.value_of("OUTPUT").is_some() {
        fs::create_dir_all(matches.value_of("OUTPUT").unwrap())?;
        fs::canonicalize(matches.value_of("OUTPUT").unwrap()).unwrap()
    } else {
        input_path.join("output")
    };

    sort_folder(input_path, output_path.as_path())?;

    Ok(())
}