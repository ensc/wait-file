#[macro_use]
extern crate log;

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::{ OsString, OsStr };
use std::io::ErrorKind;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::rc::Rc;
use std::time::Duration;

use clap::Parser;
use inotify::{Event, EventMask, Inotify, WatchDescriptor, WatchMask, Watches};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::process::{kill_process, pidfd_open, Pid, PidfdFlags, Signal};

/// Helper trait which maps empty paths to "."
trait RelPath {
    fn rel_path(&self) -> &Path;
    fn rel_parent(&self) -> Option<&Path>;
}

impl RelPath for Path {
    fn rel_path(&self) -> &Path {
        if self.as_os_str().is_empty() {
            Path::new(".")
        } else {
            self
        }
    }

    fn rel_parent(&self) -> Option<&Path> {
        if self == Path::new(".") {
            return None;
        }

        Some(self.parent()?.rel_path())
    }
}

/// In-memory state for a single watched filesystem entry.
#[derive(Default)]
struct FsState {
    /// Whether the target currently exists on disk.
    exists:	bool,
    /// Whether the target has changed since last reset.
    changed:	bool,
}

/// Logical representation of one watched path plus its state.
struct FsEntry<'a> {
    path:	&'a Path,
    state:	RefCell<FsState>,
}

impl <'a> FsEntry<'a> {
    /// Return the watched path.
    pub fn as_path(&'a self) -> &'a Path {
        self.path
    }

    /// Return whether the path exists (according to last check).
    pub fn exists(&self) -> bool {
        self.state.borrow().exists
    }

    /// Return whether the path has changed since last reset.
    pub fn changed(&self) -> bool {
        self.state.borrow().changed
    }

    /// Reset the changed flag so future events can be detected.
    pub fn reset(&self) {
        self.state.borrow_mut().changed = false;
    }

    /// Mark the entry as changed.
    pub fn mark_changed(&self) {
        self.state.borrow_mut().changed = true;
    }

    /// Ensure the appropriate inotify watches are installed for this entry.
    ///
    /// When the path exists, install a watch on the file and on its direct
    /// parent directory.
    ///
    /// When the path does not exist, find the nearest existing ancestor and
    /// watch that directory, marking the watch as either `Directory` or
    /// `Parent` depending on how far away that ancestor is.
    fn check_path(self: &Rc<Self>, map: &mut WatchMap<'a>, watches: &mut Watches)
    {
        trace!("checking path {:?}", self.as_path());

        let path = self.as_path();
        let exists = path.exists();

        self.state.borrow_mut().exists = exists;

        if exists {
            trace!("  path exists; adding watches");

            let d = watches.add(path, WatchMask::CLOSE_WRITE | WatchMask::DELETE_SELF | WatchMask::MOVE_SELF)
                .inspect_err(|e| error!("failed to watch {path:?}: {e:?}"))
                .unwrap();

            map.vec_insert(d, WatchEntry::File(self.clone()));

            if let Some(dir) = path.rel_parent() {
                let d = watches.add(dir, WatchMask::CREATE | WatchMask::MOVED_TO | WatchMask::MOVE_SELF)
                    .inspect_err(|e| error!("failed to watch {dir:?}: {e:?}"))
                    .unwrap();

                map.vec_insert(d, WatchEntry::Directory(self.clone()));
            }
        } else {
            let info = path.ancestors()
                .map(|p| p.rel_path())
                .enumerate()
                .find(|(_, p)| p.exists());

            let Some((idx, dir)) = info else {
                warn!("no parent information found for {path:?}");
                return;
            };

            let d = watches.add(dir, WatchMask::CREATE | WatchMask::MOVED_TO | WatchMask::MOVE_SELF)
                .inspect_err(|e| error!("failed to watch {dir:?}: {e:?}"))
                .unwrap();

            let e = if idx == 0 {
                trace!("  path does not exist; adding watches to direct parent directory {dir:?}");
                WatchEntry::Directory(self.clone())
            } else {
                trace!("  path does not exist; adding watches to indirect parent directory {dir:?}");
                WatchEntry::Parent(self.clone())
            };

            map.vec_insert(d, e);
        }
    }
}

impl <'a> From<&'a Path> for FsEntry<'a> {
    fn from(path: &'a Path) -> Self {
        Self { path, state: Default::default() }
    }
}

#[derive(Clone)]
enum WatchEntry<'a> {
    File(Rc<FsEntry<'a>>),
    // The directory where the entry is directly located
    Directory(Rc<FsEntry<'a>>),
    // Some parent directory of the directory where the entry is located
    Parent(Rc<FsEntry<'a>>),
}

impl std::fmt::Debug for WatchEntry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(arg0) => f.debug_tuple("File").field(&arg0.path).finish(),
            Self::Directory(arg0) => f.debug_tuple("Directory").field(&arg0.path).finish(),
            Self::Parent(arg0) => f.debug_tuple("Parent").field(&arg0.path).finish(),
        }
    }
}

#[derive(clap::Parser, Debug)]
struct Args {
    /// Watch the given path for changes; option can be specified multiple times.
    #[arg(short('w'), long, value_name("PATH"))]
    watch:		Vec<PathBuf>,

    /// Signal to terminate the program gracefully; when program is still
    /// alive after `--term-timeout`, `--sigkill` will be sent.
    #[arg(long, default_value = "15", value_parser = parse_signal, value_name("SIGNAL"))]
    sigterm:		Signal,

    /// Signal to terminate the program forcefully.  Sent when program is still
    /// alive after sending `--sigterm` and waiting for `--term-timeout`.
    #[arg(long, default_value = "9", value_parser = parse_signal, value_name("SIGNAL"))]
    sigkill:		Signal,

    /// Wait the given duration after sending the `--sigterm` signal for
    /// process termination.  When program is still alive after this time,
    /// send the `--sigkill` signal.
    #[arg(long, default_value = "2s", value_parser = parse_timespec, value_name("DURATION"))]
    term_timeout:	Timespec,

    /// Run the program only the given time and exit then
    #[arg(long, conflicts_with="oneshot", value_name("COUNT"))]
    max_restart:	Option<u32>,

    /// Run the program only once and exit then
    #[arg(long, short('1'))]
    oneshot:		Option<bool>,

    /// Wait this time before starting the program.
    #[arg(long, default_value = "500ms", value_parser = humantime::parse_duration, value_name("DURATION"))]
    settle_time:	Duration,

    /// Execute the given command when any of the watched entries is missing.
    /// Program is executed in a shell
    #[arg(long, value_name("SHCMD"))]
    build:		Option<OsString>,

    /// Program to run; when not given, wait-file exits after events have been
    /// detected
    #[arg()]
    argv:		Vec<OsString>,
}

fn parse_timespec(tm: &str) -> Result<Timespec, clap::Error> {
    humantime::parse_duration(tm)
        .map_err(|_| clap::Error::new(clap::error::ErrorKind::InvalidValue))?
        .try_into()
        .map_err(|_| clap::Error::new(clap::error::ErrorKind::ValueValidation))
}

fn parse_signal(sig: &str) -> Result<Signal, clap::Error> {
    match sig.to_lowercase().as_str() {
        "kill"	=> Ok(Signal::KILL),
        "term"	=> Ok(Signal::TERM),
        "hup"	=> Ok(Signal::HUP),
        "int"	=> Ok(Signal::INT),
        "quit"	=> Ok(Signal::QUIT),
        "abort"	=> Ok(Signal::ABORT),
        "usr1"	=> Ok(Signal::USR1),
        "usr2"	=> Ok(Signal::USR2),
        "cont"	=> Ok(Signal::CONT),
        "stop"	=> Ok(Signal::STOP),
        v	=> {
            v.parse()
                .map_err(|_| clap::Error::new(clap::error::ErrorKind::InvalidValue))
                .map(Signal::from_named_raw)?
                .ok_or_else(|| clap::Error::new(clap::error::ErrorKind::ValueValidation))
        }
    }
}

type WatchMap<'a> = HashMap<WatchDescriptor, Vec<WatchEntry<'a>>>;

/// Convenience trait for manipulating the vector values in `WatchMap`.
trait MapUpdate<'a> {
    fn vec_insert(&mut self, wd: WatchDescriptor, e: WatchEntry<'a>);
    fn vec_extend(&mut self, wd: WatchDescriptor, e: Vec<WatchEntry<'a>>);
}

impl <'a> MapUpdate<'a> for WatchMap<'a> {
    fn vec_insert(&mut self, wd: WatchDescriptor, e: WatchEntry<'a>) {
        match self.entry(wd) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(e);
            }

            Entry::Vacant(entry) => {
                entry.insert(vec![e]);
            }
        }
    }

    fn vec_extend(&mut self, wd: WatchDescriptor, e: Vec<WatchEntry<'a>>) {
        match self.entry(wd) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().extend(e);
            }

            Entry::Vacant(entry) => {
                entry.insert(e);
            }
        }
    }
}

fn init_paths<'a>(map: &mut WatchMap<'a>, watches: &mut Watches, paths: &[Rc<FsEntry<'a>>])
{
    for p in paths {
        p.reset();
        p.check_path(map, watches);
    }
}

fn check_event<'a>(map: &mut WatchMap<'a>, watches: &mut Watches, ev: Event<&OsStr>) {
    trace!("checking event {ev:?}");

    let Some((wd, entries)) = map.remove_entry(&ev.wd) else {
        // `watches.remove()` creates an extra IGNORED event
        if ev.mask != EventMask::IGNORED {
            warn!("wd {:?} from event not found", ev.wd);
        }
        return;
    };

    let mut new_entries = Vec::with_capacity(entries.len());
    let mut do_remove = ev.mask.contains(EventMask::IGNORED);

    let mut recheck = |e: Rc<FsEntry<'a>>, _ev: &Event<_>| {
        e.check_path(map, watches);
    };

    let fname = ev.name;

    for entry in entries {
        let entry = entry.clone();

        match entry {
            WatchEntry::File(e)	=> {
                debug!("event on file {:?}", e.as_path());

                if ev.mask.contains(EventMask::MOVE_SELF) || ev.mask.contains(EventMask::DELETE_SELF) {
                    do_remove = true;
                    recheck(e, &ev);
                } else if ev.mask.contains(EventMask::CLOSE_WRITE) {
                    e.mark_changed();
                    new_entries.push(WatchEntry::File(e));
                }
            }

            WatchEntry::Directory(e) if e.as_path().file_name() == fname	=> {
                debug!("event on direct parent directory of {:?}", e.as_path());

                e.mark_changed();
                recheck(e, &ev);
            }

            e @ WatchEntry::Directory(_)	=> {
                // some other, unwatched file in the directory containing a
                // watched file has been changed; no action needed, just readd
                // the entry
                new_entries.push(e);
            }

            WatchEntry::Parent(e)		=> recheck(e, &ev),
        }
    }

    if do_remove && new_entries.is_empty() {
        let wd = match map.entry(wd) {
            Entry::Occupied(entry) if entry.get().is_empty()	=> {
                trace!("  removing watch from map");
                let (wd, _) = entry.remove_entry();
                Some(wd)
            },

            Entry::Occupied(_)		=> {
                trace!("  watch readded to map");
                None
            },

            Entry::Vacant(entry)	=> {
                trace!("  orphan wd");
                Some(entry.into_key())
            }
        };

        if !ev.mask.contains(EventMask::IGNORED) && let Some(wd) = wd {
            debug!("  removing wd {wd:?} from watches");
            let _ = watches.remove(wd);
        }
    } else {
        trace!("  readding entries {new_entries:?}");
        map.vec_extend(wd, new_entries);
    }
}

/// Wrapper around a child process and its pidfd for detecting termination.
struct Process {
    child:	Child,
    pid_fd:	OwnedFd,
}

impl Process {
    pub fn run_shell(cmd: &OsStr) -> Result<(), ()> {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .status()
            .inspect_err(|e| warn!("build command terminated abnormally: {e:?}"))
            .map_err(|_| ())
            .inspect(|c| if !c.success() { warn!("build command failed: {c:?}") })?
            .success()
            .then_some(())
            .ok_or(())
    }

    pub fn new(args: &[OsString]) -> Self {
        let child = Command::new(&args[0])
            .args(&args[1..])
            .spawn()
            .expect("failed to run program");

        let pid = Pid::from_child(&child);

        let pid_fd = pidfd_open(pid, PidfdFlags::empty())
            .expect("failed to open pidfd");

        Self { child, pid_fd }
    }

    pub fn terminate(self, args: &Args) {
        let mut child = self.child;
        let pid = Pid::from_child(&child);
        let fd  = self.pid_fd;
        let res = (|| {
            if let Some(res) = child.try_wait().unwrap() {
                return res;
            }

            let _ = kill_process(pid, args.sigterm);

            let mut pfds = [
                PollFd::new(&fd, PollFlags::IN)
            ];

            let _ = poll(&mut pfds, Some(&args.term_timeout));

            if let Some(res) = child.try_wait().unwrap() {
                return res;
            }

            let _ = kill_process(pid, args.sigkill);

            child.wait().unwrap()
        })();

        if !res.success() {
            warn!("program failed: {res:?}");
        }
    }
}

impl Args {
    fn wait_for_files(&self, paths: &[Rc<FsEntry<'_>>], child: Option<BorrowedFd<'_>>) -> bool {
        let mut inotify = Inotify::init()
            .expect("Error while initializing inotify instance");

        let mut watches = inotify.watches();
        let mut map = WatchMap::new();

        init_paths(&mut map, &mut watches, paths);

        loop {
            let all_exists  = paths.iter().all(|p| p.exists());
            let any_changed = paths.iter().any(|p| p.changed());

            debug!("  exists={all_exists}, changed={any_changed}");

            if all_exists && (child.is_none() || any_changed) {
                break true;
            }

            if !all_exists && let Some(cmd) = &self.build {
                let _ = Process::run_shell(cmd);
            }

            let mut pfds = [
                PollFd::new(&inotify, PollFlags::IN),
                PollFd::new(&inotify, PollFlags::empty()), // dummy; to be replaced by child
            ];

            let pfds = if let Some(fd) = &child {
                pfds[1] = PollFd::new(fd, PollFlags::IN);
                &mut pfds[..]
            } else {
                &mut pfds[..1]
            };

            let _ = poll(pfds, None);

            if child.is_some() && pfds[1].revents().contains(PollFlags::IN) {
                break false;
            }

            let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
            loop {
                let buffer = unsafe {
                    // SAFETY: inotify will initialize the bytes it reads, and we only
                    // access the initialized portion returned by read_events()
                    core::mem::transmute::<&mut [MaybeUninit<u8>], &mut [u8]>(&mut buffer)
                };
                match inotify.read_events(buffer) {
                    Ok(events)	=> events.for_each(|ev| check_event(&mut map, &mut watches, ev)),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e)	=> panic!("fail to read events: {e:?}"),
                }
            }
        }
    }

    pub fn run(self, exit: Exit) {
        let paths: Vec<_> = self.watch.iter().map(|w| Rc::new(w.as_path().into()))
            .collect();

        debug!("watching {:?}", self.watch);

        let mut process: Option<Process> = None;
        let mut max_restart = self.max_restart
            .or_else(|| self.oneshot.and_then(|v| v.then_some(1)));

        loop {
            let run_prog = self.wait_for_files(&paths, process.as_ref().map(|p| p.pid_fd.as_fd()));

            if let Some(child) = process.take() {
                child.terminate(&self);

                if let Some(v) = &mut max_restart && *v > 0 {
                    *v -= 1;
                }
            }

            if !exit.is_running() {
                break;
            }

            if max_restart == Some(0) {
                break;
            }

            if !run_prog {
                continue;
            }

            if self.argv.is_empty() {
                break;
            }

            std::thread::sleep(self.settle_time);

            process = Some(Process::new(&self.argv));
        }
    }
}

fn main() {
    env_logger::init();

    let args = Args::parse();

    args.run(Exit::new())
}

#[cfg(not(test))]
mod test_stub {
    pub struct Exit;

    impl Exit {
        pub const fn new() -> Self {
            Self
        }

        pub const fn is_running(&self) -> bool {
            true
        }
    }
}

#[cfg(not(test))]
use test_stub as test;

#[cfg(test)]
mod test;

use test::Exit;
