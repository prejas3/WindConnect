async function start() {
  const status = document.querySelector("#status");
  try {
    const version = await window.__TAURI__.core.invoke("core_version");
    status.textContent = `wind-core ${version} · gotowy`;
  } catch (error) {
    status.textContent = "Nie udało się uruchomić wind-core.";
    console.error(error);
  }
}

start();
