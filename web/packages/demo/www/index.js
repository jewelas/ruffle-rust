import "./index.css";

import { SourceAPI, PublicAPI } from "ruffle-core";

window.RufflePlayer = PublicAPI.negotiate(
    window.RufflePlayer,
    "local",
    new SourceAPI("local")
);
const ruffle = window.RufflePlayer.newest();

let player;

const main = document.getElementById("main");
const overlay = document.getElementById("overlay");
const prompt = document.getElementById("prompt");
const authorContainer = document.getElementById("author-container");
const author = document.getElementById("author");
const sampleFileInputContainer = document.getElementById(
    "sample-swfs-container"
);
const localFileInput = document.getElementById("local-file");
const sampleFileInput = document.getElementById("sample-swfs");
const animOptGroup = document.getElementById("anim-optgroup");
const gamesOptGroup = document.getElementById("games-optgroup");

// Default config used by the player.
const config = {
    letterbox: "on",
    logLevel: "warn",
};

function unload() {
    if (player) {
        player.remove();
    }
    prompt.classList.remove("hidden");
}

function load(options) {
    unload();
    prompt.classList.add("hidden");

    player = ruffle.createPlayer();
    player.id = "player";
    main.append(player);
    player.load(options);
}

function showSample(swfData) {
    authorContainer.classList.remove("hidden");
    author.textContent = swfData.author;
    author.href = swfData.authorLink;
    localFileInput.value = null;
}

function hideSample() {
    sampleFileInput.selectedIndex = 0;
    authorContainer.classList.add("hidden");
    author.textContent = "";
    author.href = "";
}

async function loadFile(file) {
    if (!file) {
        return;
    }
    hideSample();
    load({ data: await new Response(file).arrayBuffer(), ...config });
}

function loadSample() {
    const swfData = sampleFileInput[sampleFileInput.selectedIndex].swfData;
    if (swfData) {
        showSample(swfData);
        load({ url: swfData.location, ...config });
    } else {
        hideSample();
        unload();
    }
}

localFileInput.addEventListener("change", (event) => {
    loadFile(event.target.files[0]);
});

sampleFileInput.addEventListener("change", () => loadSample());

main.addEventListener("dragenter", (event) => {
    event.stopPropagation();
    event.preventDefault();
});
main.addEventListener("dragleave", (event) => {
    event.stopPropagation();
    event.preventDefault();
    overlay.classList.remove("drag");
});
main.addEventListener("dragover", (event) => {
    event.stopPropagation();
    event.preventDefault();
    overlay.classList.add("drag");
});
main.addEventListener("drop", (event) => {
    event.stopPropagation();
    event.preventDefault();
    overlay.classList.remove("drag");
    loadFile(event.dataTransfer.files[0]);
});

window.addEventListener("load", () => {
    overlay.classList.remove("hidden");
});

(async () => {
    const response = await fetch("swfs.json");
    if (!response.ok) {
        return;
    }

    const data = await response.json();
    for (const swfData of data.swfs) {
        const option = document.createElement("option");
        option.textContent = swfData.title;
        option.value = swfData.location;
        option.swfData = swfData;
        switch (swfData.type) {
            case "Animation":
                animOptGroup.append(option);
                break;
            case "Game":
                gamesOptGroup.append(option);
                break;
        }
    }
    sampleFileInputContainer.classList.remove("hidden");

    const initialFile = new URL(window.location).searchParams.get("file");
    if (initialFile) {
        const options = Array.from(sampleFileInput.options);
        sampleFileInput.selectedIndex = Math.max(
            options.findIndex((swfData) => swfData.value.endsWith(initialFile)),
            0
        );
        loadSample();
    }
})();
