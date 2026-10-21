namespace Api_fast_bmgroup.Features.ModuloContabilidad.Dto.response;

#region "Libro Diario 5.1"
public class ReporteLibroDiarioResponse
{
    public string car { get; set; } = string.Empty;
    public string cuo { get; set; } = string.Empty;
    public string correlativo { get; set; } = string.Empty;
    public DateTime fechaOperacion { get; set; }
    public string glosa { get; set; } = string.Empty;
    public string codigoLibro { get; set; } = string.Empty;
    public string numeroDocReferencia { get; set; } = string.Empty;
    public string cuenta { get; set; } = string.Empty;
    public string denominacion { get; set; } = string.Empty;
    public decimal debe { get; set; }
    public decimal haber { get; set; }
}

public class DatosReporteLibroDiarioResponse
{
    public int totalRegistros { get; set; }
    public int totalPaginas { get; set; }
    public int pagina { get; set; }
    public int registrosPorPagina { get; set; }
    public decimal totalDebe { get; set; }
    public decimal totalHaber { get; set; }
}

public class ReporteLibroDiarioPaginadoResponse
{
    public DatosReporteLibroDiarioResponse? datos { get; set; }
    public List<ReporteLibroDiarioResponse> reporte { get; set; } = [];
}
#endregion

#region "Libro Diario 5.2"
public class MetaDataLibroDiario52
{
    public int totalCuos { get; set; }
    public int totalPaginas { get; set; }
    public int pagina { get; set; }
    public int registrosPorPagina { get; set; }
}
public class ReporteLibroDiario52
{
    public string cuo { get; set; } = string.Empty;
    public DateTime fechaOperacion { get; set; }
    public string glosa { get; set; } = string.Empty;
    public string cuenta { get; set; } = string.Empty;
    public decimal importe { get; set; }
    public string tipo { get; set; } = string.Empty;
    public int orden { get; set; }
}

public class TotalesCuenta
{
    public string cuenta { get; set; } = string.Empty;
    public decimal total { get; set; }
    public string tipo { get; set; } = string.Empty;
    public int orden { get; set; }
}

public class ReporteLibroDiario52SQL
{
    public MetaDataLibroDiario52? datos { get; set; }
    public List<ReporteLibroDiario52> reporte { get; set; } = [];
    public List<TotalesCuenta> totalesPorCuenta { get; set; } = [];
}

public class SeccionColumna
{
    public string tipo { get; set; } = string.Empty;
    public int orden { get; set; }
    public List<string> cuentas { get; set; } = [];
}

public class FilaPivotada
{
    public string cuo { get; set; } = string.Empty;
    public DateTime fecha { get; set; }
    public string glosa { get; set; } = string.Empty;
    public Dictionary<string, decimal> valores { get; set; } = [];
}

public class ReporteLibroDiario52Paginado
{
    public MetaDataLibroDiario52? datos { get; set; }
    public List<SeccionColumna> columnas { get; set; } = [];
    public List<FilaPivotada> filas { get; set; } = [];
    public Dictionary<string, decimal> totalesPeriodo { get; set; } = [];
}
#endregion

#region "Libro Diario - Export"
public class LibroDiarioExportFila
{
    public string correlativoAsiento { get; set; } = string.Empty;
    public string car { get; set; } = string.Empty;
    public string fechaOperacion { get; set; } = string.Empty;
    public string glosa { get; set; } = string.Empty;
    public string codigoLibro { get; set; } = string.Empty;
    public string numeroCorrelativo { get; set; } = string.Empty;
    public string numeroDocSustentatorio { get; set; } = string.Empty;
    public string codigo { get; set; } = string.Empty;
    public string denominacion { get; set; } = string.Empty;
    public decimal debe { get; set; }
    public decimal haber { get; set; }
}

public class LibroDiarioExportMetaData
{
    public int totalRegistros { get; set; }
    public decimal totalDebe { get; set; }
    public decimal totalHaber { get; set; }
}

public class LibroDiarioExportGenerales
{
    public string ruc { get; set; } = string.Empty;
    public string razonSocial { get; set; } = string.Empty;
    public int estado { get; set; }
    public string estadoProcesamiento { get; set; } = string.Empty;
}

public class LibroDiarioExportResponse
{
    public LibroDiarioExportGenerales general { get; set; } = new();
    public LibroDiarioExportMetaData metadata { get; set; } = new();
    public List<LibroDiarioExportFila> data { get; set; } = [];
}
#endregion

#region "Libro Diario 5.2 - Export"
public class LibroDiario52ExportFila
{
    public string cuo { get; set; } = string.Empty;
    public DateTime fechaOperacion { get; set; } = DateTime.MinValue;
    public string glosa { get; set; } = string.Empty;
    public string cuenta { get; set; } = string.Empty;
    public decimal importe { get; set; }
    public string tipo { get; set; } = string.Empty;
    public int orden { get; set; }
}

public class LibroDiario52ExportTotal
{
    public string cuenta { get; set; } = string.Empty;
    public decimal total { get; set; }
    public string tipo { get; set; } = string.Empty;
    public int orden { get; set; }
}

public class LibroDiario52ExportGenerales
{
    public string ruc { get; set; } = string.Empty;
    public string razonSocial { get; set; } = string.Empty;
    public int estado { get; set; }
    public string estadoProcesamiento { get; set; } = string.Empty;
}

public class LibroDiario52ExportResponse
{
    public LibroDiario52ExportGenerales general { get; set; } = new();
    public List<LibroDiario52ExportTotal> totales { get; set; } = new();
    public List<LibroDiario52ExportFila> data { get; set; } = [];
}
#endregion
