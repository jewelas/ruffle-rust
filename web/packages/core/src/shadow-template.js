let ruffle_shadow_tmpl = document.createElement("template");
ruffle_shadow_tmpl.innerHTML = `
    <style>
        :host {
            display: inline-block;
            /* Default width/height; this will get overridden by user styles/attributes */
            width: 550px;
            height: 400px;
            touch-action: none;
            user-select: none;
            -webkit-user-select: none;
            -webkit-tap-highlight-color: transparent;
            position: relative;
        }

        #container {
            position: relative;
            width: 100%;
            height: 100%;
            overflow: hidden;
        }

        #container canvas {
            width: 100%;
            height: 100%;
        }

        #play_button {
            position: absolute;
            width: 100%;
            height: 100%;
            cursor: pointer;
            display: none;
        }

        #play_button .icon {
            position: absolute;
            top: 50%;
            left: 50%;
            width: 90%;
            height: 90%;
            max-width: 500px;
            max-height: 500px;
            transform: translate(-50%, -50%);
        }

        #play_button:hover .icon {
            filter: brightness(1.3);
        }

        #panic {
            position: absolute;
            width: 100%;
            height: 100%;
            /* Inverted colours from play button! */
            background: linear-gradient(180deg, rgba(253,58,64,1) 0%, rgba(253,161,56,1) 100%);
            color: black;
        }

        #panic a {
            color: #37528C;
        }

        #panic-title {
            margin-top: 30px;
            text-align: center;
            font-size: 42px;
            font-weight: bold;
        }

        #panic-body {
            text-align: center;
            font-size: 20px;
            position: absolute;
            top: 100px;
            bottom: 80px;
            left: 50px;
            right: 50px;
        }

        #panic-body textarea {
            width: 100%;
            height: 100%;
        }

        #panic-footer {
            position: absolute;
            bottom: 30px;
            text-align: center;
            font-size: 20px;
            width: 100%;
        }

        #panic ul {
            margin: 35px 0 0 0;
            padding: 0;
            max-width: 100%;
            display: flex;
            list-style-type: none;
            justify-content: center;
            align-items: center;
        }

        #panic li {
            padding: 10px 50px;
        }

        #right_click_menu {
            background-color: #37528c;
            color: #FFAD33;
            border-radius: 5px;
            position: absolute;
            list-style: none;
            padding: 0;
            margin: 0;
        }

        #right_click_menu .menu_item {
            padding: 5px 10px;
        }

        #right_click_menu .menu_separator {
            padding: 5px 5px;
        }

        #right_click_menu .active {
            cursor: pointer;
            color: #FFAD33;
        }

        #right_click_menu .disabled {
            cursor: default;
            color: #94672f;
        }

        #right_click_menu .active:hover {
            background-color: #184778;
        }

        #right_click_menu hr {
            color: #FFAD33;
        }

        #right_click_menu > :first-child {
            border-top-right-radius: 5px;
            border-top-left-radius: 5px;
        }

        #right_click_menu > :last-child {
            border-bottom-right-radius: 5px;
            border-bottom-left-radius: 5px;
        }
    </style>
    <style id="dynamic_styles"></style>

    <div id="container">
        <div id="play_button"><div class="icon"><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" preserveAspectRatio="xMidYMid" viewBox="0 0 250 250" style="width:100%;height:100%;"><defs><linearGradient id="a" gradientUnits="userSpaceOnUse" x1="125" y1="0" x2="125" y2="250" spreadMethod="pad"><stop offset="0%" stop-color="#FDA138"/><stop offset="100%" stop-color="#FD3A40"/></linearGradient><g id="b"><path fill="url(#a)" d="M250 125q0-52-37-88-36-37-88-37T37 37Q0 73 0 125t37 88q36 37 88 37t88-37q37-36 37-88M87 195V55l100 70-100 70z"/><path fill="#FFF" d="M87 55v140l100-70L87 55z"/></g></defs><use xlink:href="#b"/></svg></div></div>
    </div>

    <ul id="right_click_menu" style="display: none"></ul>
`;

module.exports = ruffle_shadow_tmpl;
