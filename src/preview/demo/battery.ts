import type { BatteryView } from "../../lib/battery";
import type { Answers } from "./common";

/** Нейтральная периферия: все ступени заряда и подтверждённая зарядка. */
const BATTERY: BatteryView = {
  ready: true,
  devices: [
    { id: "lg:demo-mouse", name: "Wireless Mouse", kind: "mouse", level: 82, charging: false, source: "logitech" },
    { id: "hs:demo-headset", name: "Gaming Headset", kind: "headset", level: 46, charging: true, source: "headset" },
    { id: "bt:demo-controller", name: "Wireless Controller", kind: "gamepad", level: 18, charging: null, source: "windows" },
    { id: "bt:demo-keyboard", name: "Keyboard", kind: "keyboard", level: 7, charging: null, source: "windows" },
  ],
};

export const battery: Answers = {
  battery_state: () => BATTERY,
};
