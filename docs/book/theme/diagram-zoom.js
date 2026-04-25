// Open mermaid SVG in a new tab when clicked. The browser's native
// zoom handles the rest — pinch, scroll-wheel, Ctrl+= all work.
document.addEventListener("DOMContentLoaded", () => {
    // mermaid renders async; poll until SVGs appear.
    const tryAttach = () => {
        const diagrams = document.querySelectorAll(".mermaid");
        let attached = 0;
        diagrams.forEach((div) => {
            if (div.dataset.zoomAttached === "1") return;
            const svg = div.querySelector("svg");
            if (!svg) return;
            div.dataset.zoomAttached = "1";
            div.addEventListener("click", () => {
                const clone = svg.cloneNode(true);
                clone.removeAttribute("style");
                clone.setAttribute("width", "100%");
                clone.setAttribute("height", "100%");
                const blob = new Blob(
                    [
                        `<!DOCTYPE html><html><head><title>Diagram</title>` +
                            `<style>body{margin:0;background:#1f1f1f}` +
                            `svg{display:block;width:100vw;height:100vh}` +
                            `</style></head><body>${clone.outerHTML}</body></html>`,
                    ],
                    { type: "text/html" },
                );
                const url = URL.createObjectURL(blob);
                window.open(url, "_blank");
            });
            attached++;
        });
        if (diagrams.length > 0 && attached === diagrams.length) {
            return;
        }
        setTimeout(tryAttach, 200);
    };
    tryAttach();
});
