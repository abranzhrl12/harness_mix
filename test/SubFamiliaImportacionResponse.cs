using System.Text.Json;
using System.Text.Json.Serialization;
using Api_fast_bmgroup.Shared.Dto;

namespace Api_fast_bmgroup.Features.ModuloInventario.Dto.response;

#region Response de Errores de validacion
public class ValidarImportacionSubFamilias
{
    public int iRow { get; set; }
    public int iIndex { get; set; }
    public string tFamilia { get; set; } = string.Empty;
    public string tCodigo { get; set; } = string.Empty;
    public string tNombre { get; set; } = string.Empty;
    [JsonIgnore] public string oErrores { get; set; } = "[]";
    public List<Errores> Errores =>
        JsonSerializer.Deserialize<List<Errores>>(oErrores) ?? [];
}

public class TablaErroresImportacionSubFamilias
{
    public List<string> cabeceras { get; set; } = [];
    public List<ValidarImportacionSubFamilias> filas { get; set; } = [];
}

#endregion