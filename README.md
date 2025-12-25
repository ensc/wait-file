# wait-file

A CLI utility that monitors for changes in one or multiple files and
automatically restarts a command when those changes are detected.
This tool efficiently watches individual files using Linux inotify,
making it ideal for build automation and development workflows.

## Features

- **Efficient File Monitoring**: Uses Linux inotify for low-overhead file monitoring
- **Multi-File Support**: Watch multiple files simultaneously
- **Graceful Process Management**: Configurable signals for termination (SIGTERM then SIGKILL)
- **Creation Monitoring**:  Detects when watched files are created
- **Build Command Support**: Execute a build command when watched files are missing
- **Flexible Restart Control**: Options for oneshot mode, max restart count, and settle time
- **Simple Integration**: Works seamlessly in build pipelines and development workflows

## Use Cases

- **Build Artifact Monitoring**:  Wait for the build system to create an executable and automatically run it
- **Stamp File Monitoring**:  Watch for touch events on build stamp files and trigger post-build actions
- **Incremental Builds**:  Monitor intermediate build artifacts and trigger dependent build steps
- **Testing Workflows**:  Automatically run tests when test binaries or data files are updated
- **Continuous Integration**:  Integrate with build pipelines to trigger commands after specific artifacts are built

**Note**: This tool is designed for monitoring individual files, not entire directory trees.   For directory monitoring, consider using other tools like `inotifywait`.

## Usage

```
Usage: wait-file [OPTIONS] [ARGV]...

Arguments:
  [ARGV]...  Program to run; when not given, wait-file exits after events have been detected

Options:
  -w, --watch <PATH>             Watch the given path for changes; option can be specified multiple times
      --sigterm <SIGNAL>         Signal to terminate the program gracefully; when program is still alive after `--term-timeout`, `--sigkill` will be sent [default: 15]
      --sigkill <SIGNAL>         Signal to terminate the program forcefully.  Sent when program is still alive after sending `--sigterm` and waiting for `--term-timeout` [default: 9]
      --term-timeout <DURATION>  Wait the given duration after sending the `--sigterm` signal for process termination.  When program is still alive after this time, send the `--sigkill` signal [default: 2s]
      --max-restart <COUNT>      Run the program only the given time and exit then
  -1, --oneshot <ONESHOT>        Run the program only once and exit then [possible values: true, false]
      --settle-time <DURATION>   Wait this time before starting the program [default: 500ms]
      --build <SHCMD>            Execute the given command when any of the watched entries is missing. Program is executed in a shell
  -h, --help                     Print help
```

## Installation

```bash
cargo install wait-file
```

## How It Works

1. **Initialization**: Sets up inotify watches on specified files and their parent directories
2. **Monitoring**:  Listens for file modification, creation, and deletion events
3. **Detection**: When an event occurs on a watched file:
   - If the file exists and has changed, proceed to restart
   - If the file is missing, execute the optional `--build` command
   - If another file in the same directory changed, continue monitoring
4. **Process Management**:  Gracefully terminate the running process and start a new one
5. **Repetition**:  Continue monitoring until exit conditions are met

## Logging

Enable debug logging with the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug wait-file -w myfile -- mycommand
RUST_LOG=trace wait-file -w myfile -- mycommand
```

Supported log levels: `error`, `warn`, `info`, `debug`, `trace`

## Limitations

- **Linux Only**:   Requires Linux with inotify support
- **Directory Monitoring**:  Not designed for monitoring entire directory trees; use file-specific paths
- **Network Filesystems**: May have limitations on some network filesystems (NFS, FUSE, etc.)

## License

GPL-3.0-or-later
