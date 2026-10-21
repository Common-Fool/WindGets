using System;
using System.Globalization;
using LibreHardwareMonitor.Hardware;

namespace HwSensors
{
    /// <summary>
    /// Имена групп и запись значений — те же, что у веб-сервера LHM.
    ///
    /// Это не украшение: селекторы виджетов заданы хвостами путей вида
    /// `/Temperatures/Core (Tctl/Tdie)`, а единицы разбираются по суффиксу
    /// строки в `src-tauri/src/lhm/parse.rs`. Разойдись тут хоть одно слово —
    /// виджет замолчит, и тест `widget_selectors_are_unambiguous` это покажет.
    /// </summary>
    internal static class Units
    {
        /// <summary>Имя группы, в которую датчик попадает в дереве.</summary>
        public static string Group(SensorType type)
        {
            switch (type)
            {
                case SensorType.Voltage: return "Voltages";
                case SensorType.Current: return "Currents";
                case SensorType.Power: return "Powers";
                case SensorType.Clock: return "Clocks";
                case SensorType.Temperature: return "Temperatures";
                case SensorType.Load: return "Load";
                case SensorType.Frequency: return "Frequencies";
                case SensorType.Fan: return "Fans";
                case SensorType.Flow: return "Flows";
                case SensorType.Control: return "Controls";
                case SensorType.Level: return "Levels";
                case SensorType.Factor: return "Factors";
                case SensorType.Energy: return "Energy";
                case SensorType.Noise: return "Noise";
                case SensorType.Conductivity: return "Conductivity";
                case SensorType.Humidity: return "Humidity";
                case SensorType.Throughput: return "Throughput";
                case SensorType.TimeSpan: return "Times";

                // Data и SmallData делят одну группу и различаются единицей:
                // объёмы дисков в ГБ, видеопамять в МБ
                case SensorType.Data:
                case SensorType.SmallData:
                    return "Data";

                default: return type.ToString();
            }
        }

        /// <summary>Единица датчика без числа — она же уходит в `RawValue`.</summary>
        public static string Of(ISensor sensor)
        {
            switch (sensor.SensorType)
            {
                case SensorType.Voltage: return "V";
                case SensorType.Current: return "A";
                case SensorType.Power: return "W";
                case SensorType.Clock: return "MHz";
                case SensorType.Temperature: return "°C";
                case SensorType.Load: return "%";
                case SensorType.Frequency: return "Hz";
                case SensorType.Fan: return "RPM";
                case SensorType.Flow: return "L/h";
                case SensorType.Control: return "%";
                case SensorType.Level: return "%";
                case SensorType.Factor: return "";
                case SensorType.Data: return "GB";
                case SensorType.SmallData: return "MB";
                case SensorType.Energy: return "mWh";
                case SensorType.Noise: return "dBA";
                case SensorType.Conductivity: return "µS/cm";
                case SensorType.Humidity: return "%";
                case SensorType.TimeSpan: return "s";
                case SensorType.Throughput: return "B/s";
                default: return "";
            }
        }

        /// <summary>
        /// Значение со своей единицей.
        ///
        /// У пропускной способности единица едет за величиной — та же запись
        /// отдаётся то в KB/s, то в MB/s. Это ровно то, из-за чего разбор на
        /// стороне Rust приводит всё к байтам.
        /// </summary>
        public static string Format(ISensor sensor, float value)
        {
            var culture = CultureInfo.InvariantCulture;

            switch (sensor.SensorType)
            {
                case SensorType.Voltage:
                    return value.ToString("F3", culture) + " V";
                case SensorType.Current:
                    return value.ToString("F3", culture) + " A";
                case SensorType.Power:
                    return value.ToString("F1", culture) + " W";
                case SensorType.Clock:
                    return value.ToString("F1", culture) + " MHz";
                case SensorType.Temperature:
                    return value.ToString("F1", culture) + " °C";
                case SensorType.Load:
                case SensorType.Control:
                case SensorType.Level:
                case SensorType.Humidity:
                    return value.ToString("F1", culture) + " %";
                case SensorType.Frequency:
                    return value.ToString("F1", culture) + " Hz";
                case SensorType.Fan:
                    return value.ToString("F0", culture) + " RPM";
                case SensorType.Flow:
                    return value.ToString("F0", culture) + " L/h";
                case SensorType.Factor:
                    return value.ToString("F3", culture);
                // До мегабайта, а не до десятой гигабайта: счётчики трафика сети
                // (Data Downloaded / Uploaded) с шагом в 0,1 ГБ давали «0 B»
                // при живом трафике, а адаптер, выбираемый по наибольшему из
                // них, при равенстве нулей брался первым в дереве
                case SensorType.Data:
                    return value.ToString("F3", culture) + " GB";
                case SensorType.SmallData:
                    return value.ToString("F1", culture) + " MB";
                case SensorType.Energy:
                    return value.ToString("F0", culture) + " mWh";
                case SensorType.Noise:
                    return value.ToString("F0", culture) + " dBA";
                case SensorType.Conductivity:
                    return value.ToString("F1", culture) + " µS/cm";
                case SensorType.TimeSpan:
                    return TimeSpan.FromSeconds(value).ToString();
                case SensorType.Throughput:
                    return Throughput(value, culture);
                default:
                    return value.ToString("F1", culture);
            }
        }

        private static string Throughput(float bytesPerSecond, CultureInfo culture)
        {
            const float Kib = 1024f;
            const float Mib = 1024f * Kib;
            const float Gib = 1024f * Mib;

            if (bytesPerSecond >= Gib) return (bytesPerSecond / Gib).ToString("F1", culture) + " GB/s";
            if (bytesPerSecond >= Mib) return (bytesPerSecond / Mib).ToString("F1", culture) + " MB/s";
            if (bytesPerSecond >= Kib) return (bytesPerSecond / Kib).ToString("F1", culture) + " KB/s";
            return bytesPerSecond.ToString("F1", culture) + " B/s";
        }
    }
}
