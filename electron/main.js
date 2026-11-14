// Electron shell for the existing UI. It does not reimplement anything: it
// starts the same Rust server in headless mode, reads the URL it prints, and
// opens a Chromium window on it. The Tauri shell stays untouched.

const { app, BrowserWindow, Menu } = require("electron");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const projectRoot = path.join(__dirname, "..");
const smokeTest = process.env.ELECTRON_SMOKE === "1";

let serverProcess = null;
let mainWindow = null;
let serverUrl = null;

function binaryPath() {
  const name = process.platform === "win32" ? "lucienne.exe" : "lucienne";
  const candidates = [
    path.join(projectRoot, "target", "release", name),
    path.join(projectRoot, "target", "debug", name),
  ];
  return candidates.find((candidate) => fs.existsSync(candidate)) || null;
}

function startServer() {
  return new Promise((resolve, reject) => {
    const binary = binaryPath();
    if (!binary) {
      reject(new Error("lucienne binary not found - run `cargo build` first"));
      return;
    }

    serverProcess = spawn(binary, [], {
      cwd: projectRoot,
      env: { ...process.env, LUCIENNE_HEADLESS: "1" },
      stdio: ["ignore", "pipe", "inherit"],
    });

    let output = "";
    const onData = (chunk) => {
      output += chunk.toString();
      const match = output.match(/http:\/\/[\d.]+:\d+/);
      if (match) {
        serverUrl = `${match[0]}/dashboard/`;
        resolve(serverUrl);
      }
    };

    serverProcess.stdout.on("data", onData);
    serverProcess.on("exit", (code) => {
      if (!serverUrl) reject(new Error(`server exited with code ${code}`));
    });
    setTimeout(() => {
      if (!serverUrl) reject(new Error("server did not report a URL"));
    }, 15000);
  });
}

function stopServer() {
  if (serverProcess && serverProcess.exitCode === null) {
    serverProcess.kill();
  }
  serverProcess = null;
}

function createWindow(url) {
  mainWindow = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 1024,
    minHeight: 680,
    title: "Kaimono",
    backgroundColor: "#0f172a",
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
    },
  });

  mainWindow.loadURL(url);

  if (smokeTest) {
    mainWindow.webContents.on("did-finish-load", () => {
      console.log("SMOKE LOAD", mainWindow.webContents.getURL());
      setTimeout(async () => {
        const shot = await mainWindow.webContents.capturePage();
        fs.writeFileSync("/tmp/electron-smoke.png", shot.toPNG());
        console.log("SMOKE OK", mainWindow.webContents.getURL());
        console.log("SMOKE TEXT", await mainWindow.webContents.executeJavaScript("document.body.innerText.length"));
        console.log("SMOKE URL", mainWindow.webContents.getURL());
        app.quit();
      }, 8000);
    });
  }

  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

app.on("window-all-closed", () => app.quit());
app.on("before-quit", stopServer);
process.on("exit", stopServer);

// No File/Edit/View/Window menu bar. On macOS the system app menu remains.
Menu.setApplicationMenu(null);

app.whenReady().then(async () => {
  try {
    createWindow(await startServer());
  } catch (error) {
    console.error(error.message);
    app.quit();
  }
});
