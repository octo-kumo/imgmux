use imgmux::container::Container;
use imgmux::ops::{self, Out};
use imgmux::{inv, Res};
use std::env;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process;

const HELP: &str = "\
imgmux - multiple images in one file (current image stays a valid image)

USAGE:
  imgmux <image>... <out.mux>    create mux, or add if out.mux is a valid mux
  imgmux <image>... > out.mux    create mux on stdout when stdout is piped
  imgmux <mux>                   print metadata
  imgmux explode <mux>           extract all images to their original names
  imgmux -s=N <mux>              switch current image to index N
  imgmux -d=N <mux>              delete image N (cannot be the current one)
  imgmux -c[=N] <mux>            cycle current index by N (default 1, may be negative)

OPTIONS:
  -s, --switch=N   switch to image N
  -d, --delete=N   delete image N
  -c, --cycle[=N]  cycle current index by N
  -h, --help       show this help
  -V, --version    show version
";

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Help,
    Version,
    Info(PathBuf),
    Explode(PathBuf),
    CreateToStdout(Vec<PathBuf>),
    Images { last: PathBuf, inputs: Vec<PathBuf> },
    Delete(PathBuf, usize),
    Switch(PathBuf, usize),
    Cycle(PathBuf, i64),
}

fn set<T>(slot: &mut Option<T>, value: T, name: &str) -> Res<()> {
    if slot.is_some() {
        return Err(inv(format!("{name} given more than once")));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_value<T: std::str::FromStr>(s: &str, what: &str) -> Res<T>
where
    T::Err: std::fmt::Display,
{
    s.parse::<T>()
        .map_err(|e| inv(format!("invalid {what} '{s}': {e}")))
}

fn plan(args: &[String], stdout_tty: bool) -> Res<Mode> {
    let mut positionals: Vec<PathBuf> = Vec::new();
    let mut delete: Option<usize> = None;
    let mut switch: Option<usize> = None;
    let mut cycle: Option<i64> = None;
    let mut explode = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-h" || a == "--help" {
            return Ok(Mode::Help);
        } else if a == "-V" || a == "--version" {
            return Ok(Mode::Version);
        } else if a == "explode"
            && positionals.is_empty()
            && delete.is_none()
            && switch.is_none()
            && cycle.is_none()
        {
            explode = true;
        } else if a == "-c" || a == "--cycle" {
            set(&mut cycle, 1, "--cycle")?;
        } else if a == "-d" || a == "--delete" {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| inv("--delete requires an index"))?;
            set(&mut delete, parse_value(v, "index")?, "--delete")?;
        } else if a == "-s" || a == "--switch" {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| inv("--switch requires an index"))?;
            set(&mut switch, parse_value(v, "index")?, "--switch")?;
        } else if let Some(v) = a
            .strip_prefix("-d=")
            .or_else(|| a.strip_prefix("--delete="))
        {
            set(&mut delete, parse_value(v, "index")?, "--delete")?;
        } else if let Some(v) = a
            .strip_prefix("-s=")
            .or_else(|| a.strip_prefix("--switch="))
        {
            set(&mut switch, parse_value(v, "index")?, "--switch")?;
        } else if let Some(v) = a.strip_prefix("-c=").or_else(|| a.strip_prefix("--cycle=")) {
            set(&mut cycle, parse_value(v, "cycle delta")?, "--cycle")?;
        } else if a.starts_with('-') && a.len() > 1 {
            return Err(inv(format!("unknown option '{a}' (try --help)")));
        } else {
            positionals.push(PathBuf::from(a));
        }
        i += 1;
    }

    let ops =
        delete.is_some() as u8 + switch.is_some() as u8 + cycle.is_some() as u8 + explode as u8;
    if ops > 1 {
        return Err(inv(
            "only one of --delete, --switch, --cycle, explode may be used",
        ));
    }
    if ops == 1 {
        if positionals.len() != 1 {
            return Err(inv("operation requires exactly one mux file argument"));
        }
        let path = positionals.pop().unwrap();
        if explode {
            return Ok(Mode::Explode(path));
        }
        if let Some(n) = delete {
            return Ok(Mode::Delete(path, n));
        }
        if let Some(n) = switch {
            return Ok(Mode::Switch(path, n));
        }
        if let Some(d) = cycle {
            return Ok(Mode::Cycle(path, d));
        }
        unreachable!();
    }

    if positionals.is_empty() {
        return Ok(Mode::Help);
    }
    if !stdout_tty {
        return Ok(Mode::CreateToStdout(positionals));
    }
    if positionals.len() == 1 {
        return Ok(Mode::Info(positionals.pop().unwrap()));
    }
    let last = positionals.pop().unwrap();
    Ok(Mode::Images {
        last,
        inputs: positionals,
    })
}

fn run() -> Res<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match plan(&args, io::stdout().is_terminal())? {
        Mode::Help => print!("{HELP}"),
        Mode::Version => println!("imgmux {}", env!("CARGO_PKG_VERSION")),
        Mode::CreateToStdout(inputs) => ops::create(&inputs, Out::Stdout)?,
        Mode::Info(path) => {
            let c = Container::open(&path)?;
            let mut out = io::stdout().lock();
            ops::info(&c, &mut out)?;
        }
        Mode::Explode(path) => {
            let c = Container::open(&path)?;
            ops::explode(&c, std::path::Path::new("."))?;
        }
        Mode::Delete(path, n) => {
            let c = Container::open(&path)?;
            ops::delete(&c, n)?;
            eprintln!("deleted image {n} from {}", path.display());
        }
        Mode::Switch(path, n) => {
            let c = Container::open(&path)?;
            match ops::switch(&c, n)? {
                Some(i) => eprintln!("switched {} to image {i}", path.display()),
                None => eprintln!("image {n} is already current"),
            }
        }
        Mode::Cycle(path, d) => {
            let c = Container::open(&path)?;
            match ops::cycle(&c, d)? {
                Some(i) => eprintln!("cycled {} to image {i}", path.display()),
                None => eprintln!("cycle is a no-op (image {} already current)", c.current()),
            }
        }
        Mode::Images { last, inputs } => {
            if last.exists() {
                let c = Container::open(&last)?;
                ops::add(&c, &inputs)?;
                eprintln!("added {} image(s) to {}", inputs.len(), last.display());
            } else {
                ops::create(&inputs, Out::File(last.clone()))?;
                eprintln!("created {}", last.display());
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("imgmux: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_modes() {
        assert_eq!(
            plan(&s(&["a.png", "b.png", "out.mux"]), true).unwrap(),
            Mode::Images {
                last: "out.mux".into(),
                inputs: vec!["a.png".into(), "b.png".into()]
            }
        );
        assert_eq!(
            plan(&s(&["a.png", "b.png"]), false).unwrap(),
            Mode::CreateToStdout(vec!["a.png".into(), "b.png".into()])
        );
        assert_eq!(
            plan(&s(&["m.mux"]), true).unwrap(),
            Mode::Info("m.mux".into())
        );
        assert_eq!(
            plan(&s(&["-s=2", "m.mux"]), true).unwrap(),
            Mode::Switch("m.mux".into(), 2)
        );
        assert_eq!(
            plan(&s(&["--delete", "1", "m.mux"]), true).unwrap(),
            Mode::Delete("m.mux".into(), 1)
        );
        assert_eq!(
            plan(&s(&["-c", "m.mux"]), true).unwrap(),
            Mode::Cycle("m.mux".into(), 1)
        );
        assert_eq!(
            plan(&s(&["-c=-3", "m.mux"]), true).unwrap(),
            Mode::Cycle("m.mux".into(), -3)
        );
        assert_eq!(
            plan(&s(&["explode", "m.mux"]), true).unwrap(),
            Mode::Explode("m.mux".into())
        );
        assert_eq!(plan(&s(&[]), true).unwrap(), Mode::Help);
    }

    #[test]
    fn rejects_bad_args() {
        assert!(plan(&s(&["-s=2", "-d=1", "m.mux"]), true).is_err());
        assert!(plan(&s(&["-s=x", "m.mux"]), true).is_err());
        assert!(plan(&s(&["--nope"]), true).is_err());
        assert!(plan(&s(&["-d=1"]), true).is_err());
        assert!(plan(&s(&["-c=1", "-c=2", "m.mux"]), true).is_err());
    }
}
