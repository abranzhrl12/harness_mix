using System.Text.Json;
using System.Text.Json.Serialization;
using Api_fast_bmgroup.Shared.Dto;

namespace Api_fast_bmgroup.Features.ModuloInventario.Dto.response;

#region Response de Errores de validacion
public class ValidarImportacionFamilias
{
    public int iRow { get; set; }
    public int iIndex { get; set; }
    public string tCodigo { get; set; } = string.Empty;
    public string tNombre { get; set; } = string.Empty;
    public string tPlanContableC { get; set; } = string.Empty;
    public string tPlanContableV { get; set; } = string.Empty;
    [JsonIgnore] public string oErrores { get; set; } = "[]";
    public List<Errores> Errores =>
        JsonSerializer.Deserialize<List<Errores>>(oErrores) ?? [];
}

public class TablaErroresImportacionFamilias
{
    public List<string> cabeceras { get; set; } = [];
    public List<ValidarImportacionFamilias> filas { get; set; } = [];
}


#endregion