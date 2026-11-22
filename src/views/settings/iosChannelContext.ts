// The iOS App card is a handful of components around one `useIosChannel()`: the daemon is polled
// once, by the card, and the parts it is made of read the same state through this injection.
import { inject, provide, type InjectionKey } from "vue";
import type { useIosChannel } from "./useIosChannel";

export type IosChannel = ReturnType<typeof useIosChannel>;

const KEY: InjectionKey<IosChannel> = Symbol("iosChannel");

export function provideIosChannel(channel: IosChannel): void {
  provide(KEY, channel);
}

export function injectIosChannel(): IosChannel {
  const channel = inject(KEY);
  if (!channel) throw new Error("the iOS App card's parts need the card around them");
  return channel;
}
