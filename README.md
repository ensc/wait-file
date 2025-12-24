# wait-file

`wait-file` is a cli utility which monitors for changes in one or
multiple files and runs a command.  Besides changes in existing files
it checks also whether files are created.  When such an event is
detected, the command is (re)started after terminating an already
spawned instance. .

Typical usecases are:

- waiting for a buildsystem to create the executable and run it
  afterwards

- waiting for a buildsystem to touch a stamp file and run some actions

  waiting for changes in configuration files and rerunning a command

It is **NOT** intended for monitoring whole directory trees.

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
