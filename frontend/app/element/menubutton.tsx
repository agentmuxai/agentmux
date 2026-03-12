import clsx from "clsx";
import { createSignal, JSX } from "solid-js";
import { Button } from "./button";
import { FlyoutMenu } from "./flyoutmenu";
import "./menubutton.scss";

const MenuButton = (props: MenuButtonProps): JSX.Element => {
    const [isOpen, setIsOpen] = createSignal(false);
    return (
        <div class={clsx("menubutton", props.className)}>
            <FlyoutMenu items={props.items} onOpenChange={setIsOpen}>
                <Button
                    className="grey rounded-[3px] py-[2px] px-[2px]"
                    style={{ "border-color": isOpen() ? "var(--accent-color)" : "transparent" }}
                    title={props.title}
                >
                    <div>{props.text}</div>
                    <i class="fa-sharp fa-solid fa-angle-down" />
                </Button>
            </FlyoutMenu>
        </div>
    );
};

export { MenuButton };
