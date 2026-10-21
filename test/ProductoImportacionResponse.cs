using System.Text.Json;
using System.Text.Json.Serialization;
using Api_fast_bmgroup.Features.ModuloInventario.Dto.request;
using Api_fast_bmgroup.Shared.Dto;

namespace Api_fast_bmgroup.Features.ModuloInventario.Dto.response;

#region Response de Errores de validacion
public class ValidarImportacionProductos : ProductoImportacionRequest
{
    public int iRow { get; set; }

    // El SP devuelve la columna como "Errores" (string JSON), Dapper la mapea aquí
    private string _erroresRaw = "[]";
    public string Errores
    {
        get => _erroresRaw;
        set => _erroresRaw = string.IsNullOrWhiteSpace(value) ? "[]" : value;
    }

    // Lista deserializada para serializar al frontend como array
    public List<Errores> ErroresParsed =>
        JsonSerializer.Deserialize<List<Errores>>(Errores) ?? [];
}

public class ValidarImportacionProductosResponse : ProductoImportacionRequest
{
    public int iRow { get; set; }
    public List<Errores> Errores { get; set; } = [];

    public static ValidarImportacionProductosResponse FromDapper(ValidarImportacionProductos d) =>
        new()
        {
            iRow = d.iRow,
            iIndex = d.iIndex,
            tTipo = d.tTipo,
            tCodProducto = d.tCodProducto,
            tCodSubFamilia = d.tCodSubFamilia,
            tNombre = d.tNombre,
            tCodMarca = d.tCodMarca,
            tCodUnidadMedida = d.tCodUnidadMedida,
            lActivoFijo = d.lActivoFijo,
            tCodTipoExistencia = d.tCodTipoExistencia,
            lEnKardex = d.lEnKardex,
            nStockMinimo = d.nStockMinimo,
            nStockMaximo = d.nStockMaximo,
            tMoneda = d.tMoneda,
            tCodAfectacionIGV = d.tCodAfectacionIGV,
            nPrecioVenta = d.nPrecioVenta,
            nPrecioCompra = d.nPrecioCompra,
            tCodMetodoEval = d.tCodMetodoEval,
            tPlanContableC = d.tPlanContableC,
            tPlanContableV = d.tPlanContableV,
            nStockInicial = d.nStockInicial,
            Errores = d.ErroresParsed
        };
}

public class ValidarImportacionEquivalencias : ProductoEquivalenciaImportacionRequest
{
    public int iRow { get; set; }

    private string _erroresRaw = "[]";
    public string Errores
    {
        get => _erroresRaw;
        set => _erroresRaw = string.IsNullOrWhiteSpace(value) ? "[]" : value;
    }

    public List<Errores> ErroresParsed =>
        JsonSerializer.Deserialize<List<Errores>>(Errores) ?? [];
}

public class ValidarImportacionEquivalenciasResponse : ProductoEquivalenciaImportacionRequest
{
    public int iRow { get; set; }
    public List<Errores> Errores { get; set; } = [];

    public static ValidarImportacionEquivalenciasResponse FromDapper(ValidarImportacionEquivalencias d) =>
        new()
        {
            iRow = d.iRow,
            iIndex = d.iIndex,
            tCodProducto = d.tCodProducto,
            tCodUnidadDestino = d.tCodUnidadDestino,
            nFactor = d.nFactor,
            tUso = d.tUso,
            Errores = d.ErroresParsed
        };
}

public class TablaErroresImportacionProductos
{
    public List<string> cabeceras { get; set; } = [];
    public List<ValidarImportacionProductosResponse> filas { get; set; } = [];
    public List<string> cabecerasEquivalencias { get; set; } = [];
    public List<ValidarImportacionEquivalenciasResponse> filasEquivalencias { get; set; } = [];
}

public class ValidarImportacionProductosResultado
{
    public List<ValidarImportacionProductos> productos { get; set; } = [];
    public List<ValidarImportacionEquivalencias> equivalencias { get; set; } = [];
}

#endregion