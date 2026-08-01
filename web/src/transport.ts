export interface ChatMessage {
  id: string;
  text: string;
  mine: boolean;
}

export interface ChatSession {
  inboxId: string;
  history(): Promise<ChatMessage[]>;
  stream(
    onMessage: (message: ChatMessage) => void,
    onError: (error: unknown) => void,
  ): Promise<() => Promise<void>>;
  send(text: string): Promise<void>;
  close(): Promise<void>;
}
