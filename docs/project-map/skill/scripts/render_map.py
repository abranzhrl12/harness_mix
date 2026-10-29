"""Render an evidence-backed map JSON. This does not scan a repository."""
import argparse
import json
import math
from pathlib import Path
from xml.sax.saxutils import escape

from reportlab.lib import colors
from reportlab.lib.styles import getSampleStyleSheet, ParagraphStyle
from reportlab.lib.enums import TA_CENTER
from reportlab.lib.pagesizes import A4
from reportlab.platypus import SimpleDocTemplate, Paragraph, Spacer, Table, TableStyle, PageBreak, Flowable

INK = colors.HexColor("#16304A")
ACCENT = colors.HexColor("#167C80")
PALE = colors.HexColor("#EDF5F7")

class Graph(Flowable):
    def __init__(self, nodes):
        super().__init__()
        self.nodes = nodes
        self.width, self.height = 490, 350
    def draw(self):
        c = self.canv
        ids = {n["id"] for n in self.nodes}
        # Layout chosen for this repository's small package DAG.
        expected = {"host","mcp","cli","browser","jobs","config","skills","application","domain"}
        if ids != expected:
            raise ValueError("Adapt Graph layout for a different repository before rendering.")
        positions = {"host":(245,320),"mcp":(61,245),"cli":(184,245),
                     "browser":(307,245),"jobs":(430,245),"config":(61,160),
                     "skills":(184,160),"application":(340,100),"domain":(245,25)}
        w, h = 112, 36
        c.setLineWidth(.65)
        for n in self.nodes:
            x,y = positions[n["id"]]
            for dep in n["dependsOn"]:
                tx,ty = positions[dep]
                dx,dy = tx-x,ty-y
                length = math.hypot(dx,dy)
                ux,uy = dx/length,dy/length
                scale = min(w/2/abs(ux) if ux else 1e9, h/2/abs(uy) if uy else 1e9)
                ts = min(w/2/abs(ux) if ux else 1e9, h/2/abs(uy) if uy else 1e9)
                sx,sy = x+ux*scale,y+uy*scale
                ex,ey = tx-ux*ts,ty-uy*ts
                c.setStrokeColor(colors.HexColor("#9EB5C3"))
                c.setFillColor(colors.HexColor("#9EB5C3"))
                c.setDash(3,2) if dep in n.get("optionalDependencies",[]) else c.setDash()
                c.line(sx,sy,ex,ey)
                p=c.beginPath()
                p.moveTo(ex,ey)
                p.lineTo(ex-ux*6-uy*2.5,ey-uy*6+ux*2.5)
                p.lineTo(ex-ux*6+uy*2.5,ey-uy*6-ux*2.5)
                p.close()
                c.drawPath(p,fill=1,stroke=0)
        c.setDash()
        for n in self.nodes:
            x,y=positions[n["id"]]
            c.setFillColor(INK if n["id"]=="host" else PALE)
            c.setStrokeColor(ACCENT)
            c.roundRect(x-w/2,y-h/2,w,h,6,fill=1,stroke=1)
            c.setFillColor(colors.white if n["id"]=="host" else INK)
            c.setFont("Helvetica-Bold",8.7)
            c.drawCentredString(x,y-3,n["name"])

def validate(data):
    nodes=data["nodes"]
    ids=[n["id"] for n in nodes]
    if len(ids)!=len(set(ids)):
        raise ValueError("Duplicate node ids")
    for n in nodes:
        if not set(n["dependsOn"]).issubset(ids):
            raise ValueError("Unknown dependency")
        if len(n["hash"])!=64:
            raise ValueError("Missing source SHA-256")
    return nodes

def markdown(data):
    rows=["# "+data["title"],"",data["scope"],"","Observed: "+data["date"],"",
          "A -> B means a declared direct package dependency.","","~~~mermaid","flowchart TD"]
    for n in data["nodes"]:
        rows.append('  '+n["id"]+'["'+n["name"]+'"]')
    for n in data["nodes"]:
        for d in n["dependsOn"]:
            rows.append("  "+n["id"]+(" -.-> " if d in n.get("optionalDependencies",[]) else " --> ")+d)
    rows += ["~~~","","| Module | Responsibility | Source |","|---|---|---|"]
    rows += [f'| {n["name"]} | {n["description"]} | [{n["evidence"]}]({n["evidence"]}) |' for n in data["nodes"]]
    rows += ["","## Where to start"]
    rows += [f'- {o["task"]}: {o["start"]}; {o["then"]}. Suggested: {o["check"]}' for o in data["onboarding"]]
    rows += ["","## Coverage",data["workingTree"],data["index"]["limitations"]]
    rows += ["- "+x for x in data["unknowns"]]
    rows += ["","## Refresh","Compare source hashes, reread affected sources and regenerate views after review."]
    return "\n".join(rows)+"\n"

def render(data, output):
    styles=getSampleStyleSheet()
    styles.add(ParagraphStyle(name="TitleMap",fontName="Helvetica-Bold",fontSize=27,leading=32,textColor=INK,spaceAfter=16))
    styles.add(ParagraphStyle(name="Kicker",fontName="Helvetica-Bold",fontSize=10,leading=14,textColor=ACCENT,spaceAfter=9))
    styles.add(ParagraphStyle(name="CellMap",fontSize=8.3,leading=11,textColor=INK))
    styles.add(ParagraphStyle(name="SmallMap",fontSize=8,leading=11,textColor=INK,spaceAfter=7))
    styles["BodyText"].fontSize=10
    styles["BodyText"].leading=14
    styles["BodyText"].spaceAfter=8
    def p(text,style="BodyText"):
        return Paragraph(escape(text).replace("\n", "<br/>"),styles[style])
    def table(rows,widths):
        items=[[p(str(v),"CellMap") for v in row] for row in rows]
        t=Table(items,colWidths=widths,repeatRows=1,hAlign="LEFT")
        t.setStyle(TableStyle([("BACKGROUND",(0,0),(-1,0),INK),
            ("TEXTCOLOR",(0,0),(-1,0),colors.white),
            ("ROWBACKGROUNDS",(0,1),(-1,-1),[colors.white,PALE]),
            ("VALIGN",(0,0),(-1,-1),"TOP"),("LEFTPADDING",(0,0),(-1,-1),8),
            ("RIGHTPADDING",(0,0),(-1,-1),8),("TOPPADDING",(0,0),(-1,-1),7),
            ("BOTTOMPADDING",(0,0),(-1,-1),7),
            ("LINEBELOW",(0,0),(-1,0),1,ACCENT)]))
        # Paragraph styles own text color, so recolor header paragraphs.
        for j,v in enumerate(rows[0]):
            items[0][j]=Paragraph(escape(str(v)),ParagraphStyle(name="Header"+str(j),parent=styles["CellMap"],textColor=colors.white,fontName="Helvetica-Bold"))
        t._cellvalues=items
        return t
    story=[p("HARNESS / MAPA DEL PROYECTO","Kicker"),p("Una entrada al repositorio","TitleMap"),
           p("Observado "+data["date"]+" | 9 paquetes propios | vista de dependencias","Kicker"),
           p(data["scope"]),Graph(data["nodes"]),
           p("A -> B: A declara dependencia directa de B. La línea discontinua representa browser-cdp, opcional y habilitada por defecto.","SmallMap"),
           p("Entrada comprobada: src/main.rs -> src/cli.rs::run -> harness_cli::commands::run.","SmallMap"),
           p(data["workingTree"],"SmallMap"),PageBreak(),
           p("01 / RESPONSABILIDADES","Kicker"),p("Dónde vive cada parte","TitleMap")]
    rows=[["Módulo","Responsabilidad / fuente"]]+[[n["name"],n["description"]+"\n"+n["evidence"]] for n in data["nodes"]]
    story += [table(rows,[122,373]),Spacer(1,14),
              p("Responsabilidades documentadas en las fachadas y README de cada módulo; las dependencias se verificaron en sus manifiestos.","SmallMap"),
              p("Composición nativa: src/adapters/composition.rs. Transporte del host: src/transport/runtime.rs.","SmallMap"),PageBreak(),
              p("02 / ORIENTACIÓN","Kicker"),p("Empezar por la tarea","TitleMap"),
              p("Rutas sugeridas: son recomendaciones de navegación. Los comandos de esta página no se ejecutaron para generar el mapa.")]
    rows=[["Cambio","Punto de partida / comprobación sugerida"]]+[[o["task"],o["start"]+"\n"+o["then"]+"\n"+o["check"]] for o in data["onboarding"]]
    story += [table(rows,[155,340]),Spacer(1,14),p("Herramientas utilizadas","Heading2")]
    for tool,why in data["tools"]:
        story.append(p(tool+": "+why,"SmallMap"))
    story += [PageBreak(),p("03 / EVIDENCIA Y LÍMITES","Kicker"),p("Qué sabemos y qué falta","TitleMap"),
              p("HEAD observado: "+data["head"],"SmallMap"),
              p("Hashes SHA-256 de los manifiestos consultados. El JSON conserva el texto retornado para ocho módulos y registra una diferencia de hash en cli; el manifiesto raíz permanece en el repositorio.","SmallMap")]
    for n in data["nodes"]:
        story += [p(n["evidence"],"SmallMap"),p(n["hash"],"SmallMap")]
    story += [p("Cobertura","Heading2"),p("Índice: "+data["index"]["freshness"]+". Archivos observados: "+data["index"]["observedFiles"]+". No se reconstruyó.","SmallMap")]
    for x in data["unknowns"]:
        story.append(p("- "+x,"SmallMap"))
    story += [p("Actualizar sin redescubrir todo","Heading2"),
              p("Comparar hashes, releer fuentes cambiadas y revisar afirmaciones afectadas. Regenerar vistas juntas. Preservar notas humanas. No existe actualización automática implementada.","SmallMap")]
    output.parent.mkdir(parents=True,exist_ok=True)
    def footer(c,doc):
        c.setStrokeColor(ACCENT)
        c.line(48,43,A4[0]-48,43)
        c.setFont("Helvetica",8)
        c.setFillColor(INK)
        c.drawString(48,29,"Harness | mapa con evidencia | "+data["date"])
        c.drawRightString(A4[0]-48,29,str(doc.page))
    doc=SimpleDocTemplate(str(output),pagesize=A4,rightMargin=50,leftMargin=50,topMargin=42,bottomMargin=58,title=data["title"],author="Harness project map")
    doc.build(story,onFirstPage=footer,onLaterPages=footer)

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--data",type=Path,required=True)
    ap.add_argument("--pdf",type=Path,required=True)
    ap.add_argument("--markdown",type=Path)
    ap.add_argument("--overwrite-markdown",action="store_true")
    ap.add_argument("--previews",type=Path)
    args=ap.parse_args()
    data=json.loads(args.data.read_text(encoding="utf-8-sig"))
    validate(data)
    if args.markdown:
        if args.markdown.exists() and not args.overwrite_markdown:
            ap.error("Markdown exists; review before supplying --overwrite-markdown")
        args.markdown.parent.mkdir(parents=True,exist_ok=True)
        args.markdown.write_text(markdown(data),encoding="utf-8")
    render(data,args.pdf)
    if args.previews:
        import pymupdf
        args.previews.mkdir(parents=True,exist_ok=True)
        with pymupdf.open(args.pdf) as doc:
            for i,page in enumerate(doc):
                page.get_pixmap(matrix=pymupdf.Matrix(1.3,1.3)).save(args.previews/f"page-{i+1}.png")
            print(json.dumps({"pdf":str(args.pdf),"pages":len(doc),"nodes":len(data["nodes"]),"edges":sum(len(n["dependsOn"]) for n in data["nodes"])}))
    else:
        print(args.pdf)

if __name__=="__main__":
    main()
