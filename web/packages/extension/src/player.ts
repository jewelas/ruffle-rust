import { PublicAPI, SourceAPI, publicPath, Letterbox, LogLevel } from "ruffle-core";

const api = PublicAPI.negotiate(
    window.RufflePlayer!,
    "local",
    new SourceAPI("local"),
);
window.RufflePlayer = api;
__webpack_public_path__ = publicPath(api.config, "local");
const ruffle = api.newest()!;

let player;

// Default config used by the player.
const config = {
    letterbox: Letterbox.On,
    logLevel: LogLevel.Warn,
};

window.addEventListener("DOMContentLoaded", () => {
    // TypeScript doesn't accept window.location alone.
    const url = new URL(window.location.href);
    const swfUrl = url.searchParams.get("url");
    if (!swfUrl) {
        return;
    }

    try {
        const pathname = new URL(swfUrl).pathname;
        document.title = pathname.substring(pathname.lastIndexOf("/") + 1);
    } catch (_) {
        // Ignore URL parsing errors.
    }

    player = ruffle.createPlayer();
    player.id = "player";
    document.getElementById("main")!.append(player);

    player.load({ url: swfUrl, ...config });
});
