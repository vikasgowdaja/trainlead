const { app, BrowserWindow } = require('electron'), { spawn } = require('child_process')
let py
app.whenReady().then(() => {
  py = spawn('python', ['-m', 'uvicorn', 'app:app', '--port', '8000'], { cwd: __dirname + '/../backend', stdio: 'inherit' })
  const w = new BrowserWindow({ width: 1240, height: 820, title: 'TrainLead' })
  setTimeout(() => w.loadURL('http://127.0.0.1:8000'), 2500)
})
app.on('quit', () => py && py.kill())
