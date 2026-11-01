// DeSCENT Ground as a standalone app: finds every T-Beam on USB, serves the
// dashboard to the operator's browser, and logs every line whether or not a
// browser is open.
mod assets;
mod check;
mod chipflash;
mod chips;
mod csv;
mod decode;
mod dump;
mod flash;
mod fleet;
mod flight;
mod fswbuild;
mod framing;
mod json;
mod layout;
mod lines;
#[cfg(test)]
mod live_path;
mod localfw;
mod logfile;
mod opts;
mod replay;
mod pull;
mod radio;
mod receiver_image;
mod receivers;
mod serial;
mod settings;
mod stream;
mod stm32;
mod tiles;
mod server;
mod urlfile;
mod ws;

use server::{Bound, Ctx};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() {
    // Before anything else: the settings file, the port and the data folder are the app's,
    // and reading a log at home needs none of them.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().is_some_and(|a| a == "check") {
        std::process::exit(check::run(&argv[1..]));
    }

    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let opts = match opts::parse(&argv, opts::read_conf(&dir).as_deref()) {
        Ok(o) => o,
        Err(e) => {
            println!("{}", e);
            std::process::exit(1);
        }
    };
    if opts.help {
        print!("{}", opts::HELP);
        return;
    }
    if opts.stop {
        std::process::exit(stop(&dir, opts.port));
    }

    let on = opts.host.clone().unwrap_or_else(|| String::from("127.0.0.1"));
    let (server, port) = match server::bind(&on, opts.port) {
        Ok(Bound::Ours(s, p)) => (s, p),
        Ok(Bound::AlreadyRunning(p)) => {
            let url = format!("http://{}:{}/", opts::url_host(&on), p);
            println!("DeSCENT ChipSat is already running at {}", url);
            if opts.open_browser {
                let _ = webbrowser::open(&url);
            }
            return;
        }
        Err(e) => {
            println!("could not listen on {}: {}", on, e);
            std::process::exit(1);
        }
    };

    if let Some(stale) = opts::stale_against_source(&dir) {
        println!("WARNING: {}", stale);
    }

    // After the bind, so a second double-click or a --stop makes no folders.
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from);
    let mut data = opts::data_dir(&dir, home.as_deref());
    if let Err(e) = std::fs::create_dir_all(&data) {
        println!("cannot make {} ({}), so logs go beside the program instead", data.display(), e);
        data = dir.clone();
    }

    let (log, warn) = logfile::Log::create(&data);
    if let Some(w) = warn {
        println!("{}", w);
    }
    let log_path = log.as_ref().map(|l| l.path().to_path_buf());
    let log = Arc::new(Mutex::new(log));

    let (tx, rx) = mpsc::channel();
    let hub = Arc::new(Mutex::new(serial::Hub::new(tx)));
    hub.lock().unwrap().remember_in(&dir);
    let broadcast = ws::Broadcast::new();
    let live = Arc::new(stream::Stream::with_out(
        stream::Model::new(stream::ModelSettings::load(&dir), stream::LIVE_MAX_POINTS),
        broadcast.clone(),
    ));

    // fsw tree beside the program, if any (localfw.rs)
    let local = Arc::new(localfw::Local::new(
        fswbuild::tree(std::env::current_exe().ok().as_deref()),
        &data,
        Box::new(localfw::RealBuilder),
        localfw::Carried::this_build(),
    ));

    let ctx = Arc::new(Ctx {
        hub: hub.clone(),
        broadcast: broadcast.clone(),
        log: log.clone(),
        dir: dir.clone(),
        data: data.clone(),
        port,
        pull: pull::shared(),
        chips: Arc::new(Mutex::new(chips::Registry::load(&dir))),
        chip: chipflash::shared(),
        gate: stm32::Gate::new(),
        exe: std::env::current_exe().ok(),
        started: std::time::SystemTime::now(),
        local: local.clone(),
        live: live.clone(),
        replays: replay::Replays::start(),
    });

    // Stale and lost come from silence, so the rows are looked at again each second, as
    // often as the 1.3 page redrew its ages
    std::thread::spawn({
        let live = live.clone();
        move || loop {
            std::thread::sleep(Duration::from_secs(1));
            live.tick(logfile::now_ms() as f64);
        }
    });

    std::thread::spawn({
        let ctx = ctx.clone();
        move || server::serve(server, ctx)
    });

    std::thread::spawn({
        let hub = hub.clone();
        move || loop {
            hub.lock().unwrap().poll();
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    let url = format!("http://{}:{}/", opts::url_host(&on), port);
    // After the server thread, so anything that finds the file gets an answer from
    // /api/health. Without it the app still runs; only a launcher notices.
    if let Some(w) = urlfile::write(&dir, &url) {
        println!("{}", w);
    }
    println!("DeSCENT ChipSat {}, built {}, is at {}", env!("CARGO_PKG_VERSION"), opts::built_words(), url);
    // The start of each image's sha256, which is what the flight repo publishes for it.
    match flash::CHIPSAT_DUMP {
        Some(_) => println!("carrying the dump firmware {}", flash::CHIPSAT_DUMP_ID),
        None => println!("no dump firmware in this build, so a flight log cannot be pulled"),
    }
    let carried = flight::carried();
    for line in &carried.trouble {
        println!("{}", line);
    }
    match carried.offered().as_str() {
        "" => println!("no flight image in this build"),
        offered => println!("carrying the flight image for {}", offered),
    }
    match (local.tree(), local.origin()) {
        (Some(t), Some(o)) => println!(
            "flight software tree {}, {}; the app builds its ChipSat images from it",
            t.display(),
            localfw::origin_words(&o)
        ),
        _ => println!("no flight software tree beside this program, so the app uses the images it carries"),
    }
    match &log_path {
        Some(p) => println!("logging every line to {}", p.display()),
        None => println!("not logging to a file"),
    }
    println!("logs and pulled flight logs go in {}", data.display());
    // with no TZ or /etc/localtime this is silently UTC and day folders can be a day off
    println!("named by local time, {} (UTC{})", chrono::Local::now().format("%Y-%m-%d %H:%M"), chrono::Local::now().format("%:z"));
    // Ctrl-C ends the process outright, so the log is flushed every five seconds
    // rather than on the way out. The most a kill can cost is that tail.
    println!("Ctrl-C to stop. Closing the browser tab does not stop it.");
    if opts.open_browser {
        let _ = webbrowser::open(&url);
    } else {
        println!("Not opening a browser. Open that address yourself when you want it.");
    }

    let mut since_flush = std::time::Instant::now();
    for ev in rx {
        match ev {
            serial::Event::Line { key, t_ms, text } => take_line(&log, &live, &key, t_ms, &text),
            serial::Event::Ports => {
                broadcast.send(&ws::ports_frame());
                let ports = hub.lock().unwrap().ports();
                live.set_ports(&ports, logfile::now_ms() as f64);
            }
        }
        if since_flush.elapsed() > Duration::from_secs(5) {
            if let Ok(mut l) = log.lock() {
                if let Some(l) = l.as_mut() {
                    l.flush();
                }
            }
            since_flush = std::time::Instant::now();
        }
    }
    // Only reached if every receiver's sender has gone. Out the same way /api/quit
    // goes, so the log is flushed and the url file removed either way.
    server::shutdown(&ctx, "stopping: nothing left to read");
}

// Every line a reader thread sends: into the log, then out to the live stream. Both
// get the text as the log keeps it, so a replay of the log is what was seen live; a
// tab or a lone CR inside a line reads as a space either way.
fn take_line(log: &Mutex<Option<logfile::Log>>, live: &stream::Stream, key: &str, t_ms: u128, text: &str) {
    let text = logfile::one_field(text);
    if let Ok(mut l) = log.lock() {
        if let Some(l) = l.as_mut() {
            l.write_line(t_ms, key, &text);
        }
    }
    live.line(key, t_ms as f64, &text, true);
}

// --stop from the outside. Nothing running is not a fault of ours, but a script that
// asked for a stop and got none should hear that in the exit code.
fn stop(dir: &std::path::Path, port: Option<u16>) -> i32 {
    match server::running_port(dir, port) {
        Some(p) => match server::quit(p) {
            Ok(()) => {
                println!("stopped DeSCENT ChipSat on port {}", p);
                0
            }
            Err(e) => {
                println!("{}", e);
                1
            }
        },
        None => {
            match port {
                Some(p) => println!("nothing of ours is answering on port {}", p),
                None => println!(
                    "nothing of ours is answering on ports {}-{}, or at the port {} names",
                    server::FIRST_PORT,
                    server::LAST_PORT,
                    urlfile::NAME
                ),
            }
            1
        }
    }
}
