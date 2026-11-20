import { EventEmitter } from "node:events";
import type { BusEventName, BusEvents } from "../shared/events";

/** Typed in-process event bus shared by brain/, voice/ and the window layer. */
class Bus {
  private ee = new EventEmitter();
  constructor() {
    this.ee.setMaxListeners(50);
  }
  emit<K extends BusEventName>(name: K, payload: BusEvents[K]): void {
    this.ee.emit(name, payload);
  }
  on<K extends BusEventName>(name: K, fn: (payload: BusEvents[K]) => void): () => void {
    this.ee.on(name, fn);
    return () => this.ee.off(name, fn);
  }
  once<K extends BusEventName>(name: K, fn: (payload: BusEvents[K]) => void): void {
    this.ee.once(name, fn);
  }
}

export const bus = new Bus();
