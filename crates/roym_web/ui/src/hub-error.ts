/// A failure whose message the Hub shows as it is, with no wrapper. The
/// message is either Hub-made text in the language of the Hub, or a
/// service's text that the Hub shows as the detail of its own message. Do
/// not treat every `HubError` message as translated.
export class HubError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "HubError";
  }
}
