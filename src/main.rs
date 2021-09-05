use clap::{Arg, App, AppSettings, SubCommand};
use regex::{Regex, Captures};
use std::path::Path;
use walkdir::WalkDir;
use lazy_static::lazy_static;
use std::fs;
use std::ffi::OsStr;
use std::process::Command;
use std::io::{Read, Write, Bytes};
use html_parser::Dom;
use std::fs::File;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::Mutex;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use reqwest::blocking::Client;
use std::time::Duration;
use glob::glob;

lazy_static! {
    static ref REGEX_ISSUES: Regex = Regex::new(r"(?x)(?P<name>.*) (?P<number>\d{3}).*\(.*(?P<year>\d{4}).*\)").unwrap();
    static ref REGEX_ISSUES_THOUSANDS: Regex = Regex::new(r"(?x)(?P<name>.*) (?P<number>\d{4}).*\(.*(?P<year>\d{4}).*\)").unwrap();
    static ref REGEX_VOLUMES: Regex = Regex::new(r"(?x)(?P<name>.*) v(?P<number>\d{2}).*\(.*(?P<year>\d{4}).*\)").unwrap();
    static ref REGEX_ISSUES_SEC: Regex = Regex::new(r"(?x)(?P<name>.{17})(?P<number>\d{3}).*").unwrap();
    static ref REGEXES: Vec<&'static Regex> = vec![&REGEX_ISSUES, &REGEX_ISSUES_THOUSANDS, &REGEX_VOLUMES, &REGEX_ISSUES_SEC];
    static ref KNOWN_SERIES: Mutex<HashMap<u64, String>> = {
        let mut m = HashMap::new();
        Mutex::new(m)
    };
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

fn handle_watermark<P: AsRef<Path>>(file_path: P, folder_path: P) -> std::io::Result<bool> {
    let filename_withouth_extension = file_path.as_ref().file_stem().unwrap().to_str().unwrap();
    let extraction_path = folder_path.as_ref().join(filename_withouth_extension);

    fs::create_dir_all(&extraction_path)?;

    if file_path.as_ref().to_string_lossy().ends_with(".cbr") {
        unpack_rar(file_path.as_ref(), extraction_path.as_path())?
    } else if file_path.as_ref().to_string_lossy().ends_with(".cbz") {
        unpack_zip(file_path.as_ref(), extraction_path.as_path())?;
    }

    let mut had_watermark = false;

    for entry in glob(extraction_path.as_path().to_str().unwrap()). {
        let path = entry.path();
        let filename = path.file_name().unwrap();
        let str = filename.to_str().unwrap();
        let str_low = str.to_lowercase();

        println!("{}", str_low);

        if (str_low.starts_with("z") || (str_low.starts_with("-") && str_low.ends_with("-.jpg")) || str_low.starts_with("xtag01") || str_low.starts_with("xxx_") || str_low.starts_with("x-tag")) && !str_low.contains(" 000-") && str_low.ends_with(".jpg") {
            let extraction_path = extraction_path.as_path();
            let watermark_path = path;

            had_watermark = true;

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

            /*if file_path.as_ref().to_string_lossy().ends_with(".cbr") {
                pack_rar(file_path.as_ref(), extraction_path)?;
            } else if file_path.as_ref().to_string_lossy().ends_with(".cbz") {*/
            pack_rar(file_path.as_ref(), extraction_path)?;
            //}
        }
    }

    fs::remove_dir_all(extraction_path)?;

    Ok(had_watermark)
}

fn my_hash<T>(obj: T) -> u64
    where
        T: Hash,
{
    let mut hasher = DefaultHasher::new();
    obj.hash(&mut hasher);
    hasher.finish()
}

fn sort_file<P: AsRef<Path>>(file_path: P, captures: Captures, output_path: P) -> std::io::Result<()> {
    let mut map = KNOWN_SERIES.lock().unwrap();

    let name = &captures["name"].trim();
    let number = &captures["number"].parse::<i32>().unwrap();
    let year = if map.contains_key(&my_hash(name)) {
        map.get(&my_hash(name)).unwrap()
    } else {
        map.insert(my_hash(name), captures["year"].to_string());
        &captures["year"]
    };
    let extension = get_extension_from_filename(file_path.as_ref().to_str().unwrap()).unwrap();
    let sorted_dir_path = output_path.as_ref().join(format!("{} - {}", year, name));

    fs::create_dir_all(&sorted_dir_path)?;
    let sorted_file_path = if *number >= 1000 {
        sorted_dir_path.join(format!("{:04}.{}", number, extension))
    } else {
        sorted_dir_path.join(format!("{:03}.{}", number, extension))
    };
    fs::copy(file_path.as_ref(), sorted_file_path.as_path())?;

    let watermark_result = handle_watermark(sorted_file_path.as_path(), sorted_dir_path.as_path());

    if watermark_result.unwrap() {
        if file_path.as_ref().to_str().unwrap().contains(".cbz") {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let renamed = file_path.as_ref().to_str().unwrap().replace(".cbz", ".cbr");
            let file_path_new = Path::new(&renamed);
            fs::rename(file_path.as_ref(), file_path_new)?;
            println!("Sorted (had watermark): {}", file_path_new.to_str().unwrap());
        } else {
            println!("Sorted (had watermark): {}", file_path.as_ref().to_str().unwrap());
        }
    } else {
        println!("Sorted                : {}", file_path.as_ref().to_str().unwrap());
    }

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

fn download_file<P: AsRef<Path>>(body: &str, output_path: P, client: &Client) -> reqwest::Result<()> {
    let mut start_bytes = body.find("https://weekly2.comicfiles.ru/").unwrap_or(0);
    if start_bytes == 0 {
        start_bytes = body.find("https://getcomics.info/run.php-urls/").unwrap_or(0);
    }
    let part = &body[start_bytes..];
    let end_bytes = part.find("\"").unwrap_or(part.len());
    let path = &part[..end_bytes];

    std::thread::sleep(std::time::Duration::from_millis(150));
    println!("Downloading file: {}", output_path.as_ref().to_str().unwrap());
    let mut response = client.get(path).timeout(Duration::new(120, 0)).send();
    let mut result: reqwest::Result<bytes::Bytes>;

    let mut i = 0;
    loop {
        if response.is_ok() {
            result = response.unwrap().bytes();
            if result.is_ok() {
                break;
            } else {
                println!("Failed to download file or decode bytes.");
                if i >= 5 {
                    println!("Canceling download.");
                    return Ok(())
                } else {
                    println!("Retrying.");
                    response = client.get(path).timeout(Duration::new(120, 0)).send();
                }
            }
        } else {
            println!("Failed to download file or decode bytes.");
            if i >= 5 {
                println!("Canceling download.");
                return Ok(())
            } else {
                println!("Retrying.");
                response = client.get(path).timeout(Duration::new(120, 0)).send();
            }
        }
        i += 1;
    }

    let result = result.unwrap();

    let mut file = match File::create(&output_path) {
        Err(why) => panic!("couldn't create {}", why),
        Ok(file) => file,
    };

    let res = file.write_all(&result);

    if res.is_err() {
        println!("err: {}", res.err().unwrap());
    }

    println!("File downloaded: {}", output_path.as_ref().to_str().unwrap());

    Ok(())
}

fn download_issues<P: AsRef<Path>>(input_url: &str, output_path: P) -> reqwest::Result<()> {
    let client = Client::new();

    let url_str = input_url.to_string();
    let mut year = url_str[url_str.len()-5..url_str.len()-1].parse::<i32>().unwrap();
    let url_str_before_year = &url_str[..url_str.len()-6];
    let mut number = url_str_before_year[url_str_before_year.rfind('-').unwrap()+1..].parse::<i32>().unwrap();
    let url_str_before_number = &url_str_before_year[..url_str_before_year.rfind('-').unwrap()];
    let name = &url_str_before_number[url_str_before_number.rfind('/').unwrap()+1..];

    fs::create_dir_all(&output_path);

    let body = client.get(input_url).timeout(Duration::new(120, 0))
        .send()?.text()?;
    if body.to_lowercase().contains("not found") {
        println!("File not found!");
        return Ok(())
    }

    let start_bytes = body.find("<h1 class=\"post-title\">").unwrap_or(0);
    let part = &body[start_bytes+23..];
    let end_bytes = part.find("#").unwrap_or(part.len());
    let real_name = &part[..end_bytes].trim();

    let end_path = if number >= 1000 {
        output_path.as_ref().join(&*format!("{} {:04} ({}).cbr", real_name, number, year))
    } else {
        output_path.as_ref().join(&*format!("{} {:03} ({}).cbr", real_name, number, year))
    };
    download_file(&*body, end_path, &client);

    loop {
        number = number + 1;
        let mut url_str = format!("https://getcomics.info/dc/{}-{}-{}/", name, number, year);
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut body = client.get(url_str).timeout(Duration::new(120, 0))
            .send()?.text()?;;
        if body.to_lowercase().contains("not found") {
            year = year + 1;
            url_str = format!("https://getcomics.info/dc/{}-{}-{}/", name, number, year);
            std::thread::sleep(std::time::Duration::from_millis(150));
            body = client.get(url_str).timeout(Duration::new(120, 0))
                .send()?.text()?;
            if body.to_lowercase().contains("not found") {
                year = year - 2;
                url_str = format!("https://getcomics.info/dc/{}-{}-{}/", name, number, year);
                std::thread::sleep(std::time::Duration::from_millis(150));
                body = client.get(url_str).timeout(Duration::new(120, 0))
                    .send()?.text()?;
                if body.to_lowercase().contains("not found") {
                    break;
                }
            }
        }
        let end_path = if number >= 1000 {
            output_path.as_ref().join(&*format!("{} {:04} ({}).cbr", real_name, number, year))
        } else {
            output_path.as_ref().join(&*format!("{} {:03} ({}).cbr", real_name, number, year))
        };
        download_file(&*body, end_path, &client);
    }

    Ok(())
}

fn main() -> std::io::Result<()> {
    let matches = App::new("comics")
        .version("1.0")
        .author("KINDEL Hugo <me@hugokindel.xyz>")
        .about("Handle all sorts of comics utilities")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(SubCommand::with_name("sort")
            .about("Sort a comics folder recursively")
            .arg(Arg::with_name("INPUT")
                .help("Sets the input folder to sort")
                .required(true))
            .arg(Arg::with_name("OUTPUT")
                .short("o")
                .long("output")
                .help("Sets the output folder to use")
                .takes_value(true)))
        .subcommand(SubCommand::with_name("download")
            .about("Download comics automatically")
            .arg(Arg::with_name("INPUT")
                .help("Sets the input URL to start downloading")
                .required(true)))
        .get_matches();

    if matches.subcommand_matches("sort").is_some() {
        let input_path_temp = fs::canonicalize(matches.subcommand_matches("sort").unwrap().value_of("INPUT").unwrap()).unwrap();
        let input_path = input_path_temp.as_path();
        let output_path = if matches.subcommand_matches("sort").unwrap().is_present("OUTPUT") {
            fs::create_dir_all(matches.subcommand_matches("sort").unwrap().value_of("OUTPUT").unwrap())?;
            fs::canonicalize(matches.subcommand_matches("sort").unwrap().value_of("OUTPUT").unwrap()).unwrap()
        } else {
            input_path.join("output")
        };

        sort_folder(input_path, output_path.as_path())?;
    } else if matches.subcommand_matches("download").is_some() {
        let input_path_temp = fs::canonicalize(".").unwrap();
        let input_path = input_path_temp.as_path();
        let output_path = if matches.subcommand_matches("download").unwrap().is_present("OUTPUT") {
            fs::create_dir_all(matches.subcommand_matches("download").unwrap().value_of("OUTPUT").unwrap())?;
            fs::canonicalize(matches.subcommand_matches("download").unwrap().value_of("OUTPUT").unwrap()).unwrap()
        } else {
            input_path.join("output")
        };

        let output = output_path.as_path();

        download_issues(matches.subcommand_matches("download").unwrap().value_of("INPUT").unwrap(), output);
        sort_folder(output, output.join("output").as_path())?;
    }

    Ok(())
}