using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text;
using LibreHardwareMonitor.Hardware;

namespace HwSensors
{
    /// <summary>
    /// Сайдкар телеметрии: читает железо через LibreHardwareMonitorLib и отдаёт
    /// снимок в stdout.
    ///
    /// Формат провода — тот же, что у веб-сервера LibreHardwareMonitor
    /// (`data.json`), намеренно. На стороне Rust уже написаны разбор дерева,
    /// нормализация единиц и селекторы виджетов, они покрыты тестами и снятой
    /// картой датчиков. Свой формат означал бы переписать всё
    /// это разом ради одной лишь красоты.
    ///
    /// Такт задаёт Rust: пишет строку `poll` в stdin и читает строку из stdout.
    /// Шаг опроса живёт в настройках приложения, и знать о нём сайдкару незачем.
    /// Конец stdin означает «приложение ушло» — выходим.
    /// </summary>
    internal static class Program
    {
        /// <summary>
        /// Точка входа. Больше здесь ничего и не делается: тела метода касается
        /// JIT перед первой строкой, и упомяни оно типы LibreHardwareMonitor —
        /// их сборку искали бы раньше, чем встанет обработчик, который её и
        /// достаёт. Поэтому вся работа в <see cref="Run"/>, а он не встраивается.
        /// </summary>
        private static int Main(string[] args)
        {
            AppDomain.CurrentDomain.AssemblyResolve += FromEmbedded;
            return Run(args);
        }

        /// <summary>Достаёт вшитую в exe сборку по имени.</summary>
        private static Assembly FromEmbedded(object sender, ResolveEventArgs args)
        {
            var name = new AssemblyName(args.Name).Name + ".dll";
            using (var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream(name))
            {
                if (stream == null) return null;

                // Читаем до конца, а не одним вызовом: Read вправе вернуть
                // меньше запрошенного, и недочитанная сборка не загрузится
                using (var whole = new MemoryStream())
                {
                    stream.CopyTo(whole);
                    return Assembly.Load(whole.ToArray());
                }
            }
        }

        [MethodImpl(MethodImplOptions.NoInlining)]
        private static int Run(string[] args)
        {
            // Числа и разделитель обязаны быть предсказуемыми: у веб-сервера LHM
            // они по локали системы, и Rust умеет обе записи, но выбирать между
            // ними на каждом запуске незачем
            CultureInfo.DefaultThreadCurrentCulture = CultureInfo.InvariantCulture;

            var output = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            output.AutoFlush = false;

            var computer = new Computer
            {
                IsCpuEnabled = true,
                IsGpuEnabled = true,
                IsMemoryEnabled = true,
                IsMotherboardEnabled = true,
                IsStorageEnabled = true,
                IsNetworkEnabled = true,
                // Контроллеры охлаждения нужны ради оборотов вентиляторов
                IsControllerEnabled = true,
                // Батареи и псевдоустройства этой машине не нужны
                IsBatteryEnabled = false,
                IsPsuEnabled = false,
            };

            try
            {
                computer.Open();
            }
            catch (Exception error)
            {
                Console.Error.WriteLine("не удалось открыть железо: " + error.Message);
                return 1;
            }

            var visitor = new UpdateVisitor();
            // Снимок весит десятки килобайт: свежий буфер на каждом такте ложился
            // бы в кучу больших объектов, которую .NET Framework не уплотняет
            var json = new StringBuilder(64 * 1024);
            var chunk = new char[4096];
            var once = Array.IndexOf(args, "--once") >= 0;

            try
            {
                do
                {
                    if (!once)
                    {
                        var command = Console.In.ReadLine();
                        // Конец потока — приложение закрылось, держаться не за что
                        if (command == null) break;
                        if (command.Trim().Length == 0) continue;
                    }

                    computer.Accept(visitor);
                    Snapshot(json, computer, visitor.Stale);
                    // Кусками через один массив: `ToString` выделял бы ту же
                    // большую строку заново, а `Write(StringBuilder)` в net472 нет
                    for (var at = 0; at < json.Length; at += chunk.Length)
                    {
                        var count = Math.Min(chunk.Length, json.Length - at);
                        json.CopyTo(at, chunk, 0, count);
                        output.Write(chunk, 0, count);
                    }
                    output.Write('\n');
                    output.Flush();
                } while (!once);
            }
            finally
            {
                computer.Close();
            }

            return 0;
        }

        /// <summary>
        /// Собирает дерево снимка: корень «Sensor», под ним имя машины, под ним
        /// железо. Первый сегмент пути — имя машины, ровно как у веб-сервера.
        /// </summary>
        private static void Snapshot(StringBuilder json, Computer computer, HashSet<IHardware> stale)
        {
            json.Clear();
            var id = 0;

            json.Append("{\"id\":").Append(id++).Append(",\"Text\":\"Sensor\",\"Children\":[");
            json.Append("{\"id\":").Append(id++).Append(",\"Text\":");
            Quote(json, Environment.MachineName);
            json.Append(",\"Children\":[");

            var first = true;
            foreach (var hardware in computer.Hardware)
            {
                if (!first) json.Append(',');
                first = false;
                Hardware(json, hardware, ref id, stale);
            }

            json.Append("]}]}");
        }

        /// <summary>
        /// Узел железа. Сверх формата веб-сервера у него два поля — вид железа
        /// и его идентификатор (этап 57).
        ///
        /// Виджеты по ним сперва находят железо, а потом датчик внутри него:
        /// по одному хвосту пути на машине со встройкой и дискретной
        /// видеокартой обе отдают «Load/GPU Core», и первым по порядку могла
        /// оказаться встройка. Вид — публичное перечисление библиотеки, а не
        /// разбор `SensorId`; идентификатор нужен ради встройки Intel, у
        /// которой вид тот же, что у дискретной.
        /// </summary>
        private static void Hardware(StringBuilder json, IHardware hardware, ref int id, HashSet<IHardware> stale)
        {
            json.Append("{\"id\":").Append(id++).Append(",\"Text\":");
            Quote(json, hardware.Name);
            json.Append(",\"HardwareType\":");
            Quote(json, hardware.HardwareType.ToString());
            json.Append(",\"HardwareId\":");
            Quote(json, hardware.Identifier.ToString());
            json.Append(",\"Children\":[");

            var first = true;
            var frozen = stale.Contains(hardware);

            // Подчинённое железо идёт раньше групп датчиков: у памяти так
            // получается сегмент «Total Memory», на который смотрят селекторы
            foreach (var sub in hardware.SubHardware)
            {
                if (!first) json.Append(',');
                first = false;
                Hardware(json, sub, ref id, stale);
            }

            foreach (var group in Groups(hardware))
            {
                if (!first) json.Append(',');
                first = false;

                json.Append("{\"id\":").Append(id++).Append(",\"Text\":");
                Quote(json, group.Key);
                json.Append(",\"Children\":[");

                var firstSensor = true;
                foreach (var sensor in group.Value)
                {
                    if (!firstSensor) json.Append(',');
                    firstSensor = false;
                    Sensor(json, sensor, ref id, frozen);
                }

                json.Append("]}");
            }

            json.Append("]}");
        }

        /// <summary>
        /// Один датчик снимка.
        ///
        /// Полей здесь на два меньше, чем отдавал веб-сервер LHM: «Min» и «Max»
        /// не читает ни Rust (их нет в структуре Node), ни виджеты. А стоили
        /// они по два форматирования на датчик в каждом такте — при 233
        /// датчиках и опросе раз в секунду это под пятьсот лишних строк в
        /// секунду и примерно треть длины ответа, круглые сутки в никуда.
        /// Совместимость формата от этого не страдает: лишние поля разбор
        /// пропускает, недостающие — тоже, пока их никто не спрашивает.
        /// </summary>
        private static void Sensor(StringBuilder json, ISensor sensor, ref int id, bool frozen)
        {
            var unit = Units.Of(sensor);

            json.Append("{\"id\":").Append(id++).Append(",\"Text\":");
            Quote(json, sensor.Name);
            json.Append(",\"SensorId\":");
            Quote(json, sensor.Identifier.ToString());
            json.Append(",\"Type\":");
            Quote(json, sensor.SensorType.ToString());

            // Пропуск значения — прочерк, ровно как у веб-сервера. Единица при
            // этом остаётся в RawValue, и по ней виджет знает, что показывать.
            // У железа, не обновившегося на этом такте, значения прошлые, — тоже
            // прочерк: иначе карточка выдавала бы замёрзшее число за живое
            json.Append(",\"Value\":");
            Quote(json, !frozen && sensor.Value.HasValue ? Units.Format(sensor, sensor.Value.Value) : "-");
            json.Append(",\"RawValue\":");
            Quote(json, " " + unit);
            json.Append('}');
        }

        /// <summary>
        /// Раскладывает датчики железа по группам так же, как веб-сервер:
        /// «Temperatures», «Load», «Clocks» и прочие. Группировка идёт по имени,
        /// а не по типу: Data и SmallData оба зовутся «Data», и разными группами
        /// они разъехались бы надвое.
        /// </summary>
        private static List<KeyValuePair<string, List<ISensor>>> Groups(IHardware hardware)
        {
            var order = new List<string>();
            var groups = new Dictionary<string, List<ISensor>>();

            foreach (var sensor in hardware.Sensors)
            {
                var name = Units.Group(sensor.SensorType);
                if (!groups.TryGetValue(name, out var list))
                {
                    list = new List<ISensor>();
                    groups[name] = list;
                    order.Add(name);
                }

                list.Add(sensor);
            }

            var result = new List<KeyValuePair<string, List<ISensor>>>(order.Count);
            foreach (var name in order)
            {
                result.Add(new KeyValuePair<string, List<ISensor>>(name, groups[name]));
            }

            return result;
        }

        /// <summary>Пишет строку в JSON с экранированием.</summary>
        private static void Quote(StringBuilder json, string text)
        {
            json.Append('"');
            foreach (var ch in text ?? string.Empty)
            {
                switch (ch)
                {
                    case '"': json.Append("\\\""); break;
                    case '\\': json.Append("\\\\"); break;
                    case '\n': json.Append("\\n"); break;
                    case '\r': json.Append("\\r"); break;
                    case '\t': json.Append("\\t"); break;
                    default:
                        if (ch < ' ')
                        {
                            json.Append("\\u").Append(((int)ch).ToString("x4", CultureInfo.InvariantCulture));
                        }
                        else
                        {
                            json.Append(ch);
                        }

                        break;
                }
            }

            json.Append('"');
        }
    }

    /// <summary>
    /// Обход дерева железа с обновлением показаний. Подчинённое железо обновляем
    /// тоже: у памяти и платы датчики живут именно там.
    ///
    /// Сбой одного устройства — не сбой опроса: исключение из `Update` иначе
    /// роняло бы сайдкар, и упрямо сбоящий диск гонял бы его по кругу
    /// «упал — подняли». Датчики устройства на таком такте уходят прочерками
    /// (<see cref="Stale"/>), в журнал сбой попадает один раз за запуск.
    /// </summary>
    internal sealed class UpdateVisitor : IVisitor
    {
        private readonly HashSet<string> failed = new HashSet<string>();

        /// <summary>Железо, чей `Update` упал на нынешнем такте.</summary>
        public readonly HashSet<IHardware> Stale = new HashSet<IHardware>();

        public void VisitComputer(IComputer computer)
        {
            Stale.Clear();
            computer.Traverse(this);
        }

        public void VisitHardware(IHardware hardware)
        {
            try
            {
                hardware.Update();
            }
            catch (Exception error)
            {
                Stale.Add(hardware);
                if (failed.Add(hardware.Identifier.ToString()))
                    Console.Error.WriteLine("не обновилось железо " + hardware.Name + ": " + error.Message);
            }
            foreach (var sub in hardware.SubHardware) sub.Accept(this);
        }

        public void VisitSensor(ISensor sensor) { }

        public void VisitParameter(IParameter parameter) { }
    }
}
