package main

import (
    "errors"
    "fmt"
    "net"
    "os"
    "os/exec"
    "os/signal"
    "path/filepath"
    "syscall"
    "time"
)

const socketName = "launcher.sock"
const drawerReopenDelay = time.Second

func socketPath() string {
    runtimeDir := os.Getenv("XDG_RUNTIME_DIR")
    if runtimeDir == "" {
        fmt.Fprintln(os.Stderr, "XDG_RUNTIME_DIR is not set")
        os.Exit(1)
    }

    return filepath.Join(runtimeDir, socketName)
}

func userHomePath() string {
    homeDir, err := os.UserHomeDir()
    if err != nil {
        fmt.Fprintln(os.Stderr, "Failed to determine home directory: %w", err)
        os.Exit(1)
    }

    return homeDir
}

func appStartPath(appID string) string {
    return filepath.Join(userHomePath(), ".local/bin/sieve/desktop-manager", appID)
}

func listenForLaunches(socketPath string, requests chan<- string) (net.Listener, error) {
    _ = os.Remove(socketPath)

    listener, err := net.Listen("unix", socketPath)
    if err != nil {
        return nil, err
    }

    go func() {
        for {
            conn, err := listener.Accept()
            if err != nil {
                return
            }

            go func() {
                defer conn.Close()

                buffer := make([]byte, 4096)

                n, err := conn.Read(buffer)
                if err != nil {
                    return
                }

                appID := string(buffer[:n])

                _, _ = conn.Write([]byte("ok"))

                requests <- appID
            }()
        }
    }()

    return listener, nil
}

func runDrawer() <-chan error {
    done := make(chan error, 1)

    go func() {
        fmt.Println("Opening launcher")

        cmd := exec.Command(
            "/run/current-system/sw/bin/nwg-drawer",
            "-c", "5",
            "-is", "64",
            "-spacing", "20",
            "-nofs",
            "-nocats",
            "-pbuseicontheme",
            "-pbexit", "logout",
            "-pbpoweroff", "poweroff",
        )

        cmd.Stdout = os.Stdout
        cmd.Stderr = os.Stderr

        done <- cmd.Run()
    }()

    return done
}

func runApplication(appID string) error {
    startPath := appStartPath(appID)
    if _, err := os.Stat(startPath); errors.Is(err, os.ErrNotExist) {
        return fmt.Errorf("Unknown application requested: %s", appID)
    }

    fmt.Printf(
        "Starting application: %s (%s)\n",
        appID,
        startPath,
    )

    cmd := exec.Command(startPath)

    cmd.Stdout = os.Stdout
    cmd.Stderr = os.Stderr
    cmd.Stdin = os.Stdin

    cmd.SysProcAttr = &syscall.SysProcAttr{
        Setpgid: true,
    }

    if err := cmd.Start(); err != nil {
        return err
    }

    fmt.Printf(
        "Application %s started with PID %d\n",
        appID,
        cmd.Process.Pid,
    )

    err := cmd.Wait()

    fmt.Printf("Application %s exited\n", appID)

    return err
}

func sendLaunchRequest(appID string) error {
    conn, err := net.Dial("unix", socketPath())
    if err != nil {
        return fmt.Errorf("could not connect to launcher: %w", err)
    }

    defer conn.Close()

    if _, err := conn.Write([]byte(appID)); err != nil {
        return err
    }

    buffer := make([]byte, 16)

    _, err = conn.Read(buffer)

    return err
}

func logout() error {
    fmt.Println("Logging out")

    cmd := exec.Command(
        "/run/current-system/sw/bin/loginctl",
        "terminate-user",
        os.Getenv("USER"),
    )

    cmd.Stdout = os.Stdout
    cmd.Stderr = os.Stderr

    return cmd.Run()
}

func main() {
    // Internal command used by the nwg-drawer logout button.
    if len(os.Args) >= 2 && os.Args[1] == "logout" {
        if err := logout(); err != nil {
            fmt.Fprintf(os.Stderr, "Logout failed: %v\n", err)
            os.Exit(1)
        }

        return
    }

    // Command used by .desktop files to request an application launch.
    if len(os.Args) >= 2 && os.Args[1] == "launch" {
        if err := sendLaunchRequest(os.Args[2]); err != nil {
            fmt.Fprintln(os.Stderr, err)
            os.Exit(1)
        }

        return
    }

    socket := socketPath()

    requests := make(chan string, 1)

    listener, err := listenForLaunches(socket, requests)
    if err != nil {
        fmt.Fprintf(
            os.Stderr,
            "Failed to create socket: %v\n",
            err,
        )
        os.Exit(1)
    }

    defer listener.Close()
    defer os.Remove(socket)

    signals := make(chan os.Signal, 1)
    signal.Notify(signals, syscall.SIGTERM, syscall.SIGINT)

    go func() {
        <-signals
        listener.Close()
        os.Remove(socket)
        os.Exit(0)
    }()

    fmt.Println("Launcher started")
    fmt.Printf("Socket: %s\n", socket)

    for {
        drawerDone := runDrawer()

        var appID string
        var haveRequest bool
        var drawerClosed bool
        var reopenDrawer <-chan time.Time

    wait:
        for !haveRequest || !drawerClosed {
            select {
            case appID = <-requests:
                fmt.Printf("Launch requested: %s\n", appID)
                haveRequest = true

            case err := <-drawerDone:
                drawerClosed = true

                if err != nil {
                    fmt.Printf(
                        "nwg-drawer exited: %v\n",
                        err,
                    )
                } else {
                    fmt.Println("nwg-drawer closed")
                }

                if !haveRequest {
                    reopenDrawer = time.After(drawerReopenDelay)
                }

            case <-reopenDrawer:
                fmt.Println("No launch request after drawer closed, reopening")
                break wait
            }
        }

        if !haveRequest {
            continue
        }

        if err := runApplication(appID); err != nil {
            var exitErr *exec.ExitError

            if errors.As(err, &exitErr) {
                fmt.Printf(
                    "%s exited with status %d\n",
                    appID,
                    exitErr.ExitCode(),
                )
            } else {
                fmt.Printf(
                    "Error running %s: %v\n",
                    appID,
                    err,
                )
            }
        }
    }
}
