import { interdict } from "../../js-src/interdiction";
import { get_config_options } from "../../js-src/config";

let html = document.getElementsByTagName("html")[0];
let page_options = get_config_options(html);
let interdictions = page_options.interdict;
interdict(interdictions);