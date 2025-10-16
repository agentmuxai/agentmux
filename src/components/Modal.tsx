import { Component, JSX, Show } from 'solid-js';

interface ModalProps {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  children: JSX.Element;
}

const Modal: Component<ModalProps> = (props) => {
  const handleOverlayClick = (e: MouseEvent) => {
    if (e.target === e.currentTarget) {
      props.onClose();
    }
  };

  const handleEscape = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      props.onClose();
    }
  };

  return (
    <Show when={props.isOpen}>
      <div
        class="modal-overlay"
        onClick={handleOverlayClick}
        onKeyDown={handleEscape}
        data-testid="modal-overlay"
      >
        <div class="modal-container" data-testid="modal-container">
          <div class="modal-header">
            <h2 class="modal-title">{props.title}</h2>
            <button
              class="modal-close"
              onClick={props.onClose}
              aria-label="Close modal"
              data-testid="modal-close"
            >
              ✕
            </button>
          </div>
          <div class="modal-content">
            {props.children}
          </div>
        </div>
      </div>
    </Show>
  );
};

export default Modal;
