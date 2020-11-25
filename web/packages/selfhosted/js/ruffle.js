const { PublicAPI, SourceAPI, publicPath } = require("ruffle-core");

window.RufflePlayer = PublicAPI.negotiate(
    window.RufflePlayer,
    "local",
    new SourceAPI("local")
);
__webpack_public_path__ = publicPath(window.RufflePlayer.config, "local");
